use super::*;

impl<'a> Model<'a> {
    pub(super) fn type_is_retained(&self, ty: &syn::Type, module: &[String]) -> bool {
        match ty {
            syn::Type::Path(path) if path.qself.is_none() => {
                let Some(last) = path.path.segments.last() else {
                    return false;
                };
                let name = last.ident.to_string();
                if is_primitive(&name) {
                    return true;
                }
                if let Some(key) = self.resolve_source(&path.path, module) {
                    return self.structs.contains_key(&key)
                        || self
                            .enums
                            .get(&key)
                            .is_some_and(|info| info.unit || info.portable_value);
                }
                if matches!(name.as_str(), "Option" | "Vec" | "Box") {
                    return one_type_argument(last)
                        .is_some_and(|inner| self.type_is_retained(inner, module));
                }
                false
            }
            syn::Type::Array(array) => self.type_is_retained(&array.elem, module),
            syn::Type::Tuple(tuple) if tuple.elems.len() <= 12 => {
                !tuple.elems.is_empty()
                    && tuple
                        .elems
                        .iter()
                        .all(|element| self.type_is_retained(element, module))
            }
            syn::Type::Paren(paren) => self.type_is_retained(&paren.elem, module),
            _ => false,
        }
    }

    pub(super) fn type_is_fresh(&self, ty: &syn::Type, module: &[String]) -> bool {
        match ty {
            syn::Type::BareFn(_) => true,
            syn::Type::TraitObject(object) => object.bounds.iter().any(bound_is_callback),
            syn::Type::ImplTrait(object) => object.bounds.iter().any(bound_is_callback),
            syn::Type::Path(path) if path.qself.is_none() => {
                if self.resolve_source(&path.path, module).is_some() {
                    return false;
                }
                let Some(last) = path.path.segments.last() else {
                    return false;
                };
                let name = last.ident.to_string();
                // Runtime controller handles own native resources and cannot be
                // serialized into the portable state bundle. They are fresh
                // configuration when imported from a framework crate; source
                // types were resolved above and still take their normal path.
                if name == "StateUpdater"
                    || name.contains("Callback")
                    || name.ends_with("Handler")
                    || name.ends_with("Controller")
                {
                    return true;
                }
                // Interior-mutable cells and their shared ownership wrappers
                // describe generation-local runtime state, not a portable
                // value. Treat the cell itself as fresh, and propagate that
                // classification through Rc/Arc/Weak so fields such as
                // Rc<Cell<PointerState>> do not block a callback rebuild.
                if matches!(
                    name.as_str(),
                    "Cell"
                        | "RefCell"
                        | "UnsafeCell"
                        | "OnceCell"
                        | "OnceLock"
                        | "Mutex"
                        | "RwLock"
                ) {
                    return true;
                }
                if matches!(name.as_str(), "Rc" | "Arc" | "Weak") {
                    return one_type_argument(last)
                        .is_some_and(|inner| self.type_is_fresh(inner, module));
                }
                matches!(name.as_str(), "Option" | "Vec" | "Box")
                    && one_type_argument(last)
                        .is_some_and(|inner| self.type_is_fresh(inner, module))
            }
            syn::Type::Array(array) => self.type_is_fresh(&array.elem, module),
            syn::Type::Tuple(tuple) => tuple
                .elems
                .iter()
                .any(|element| self.type_is_fresh(element, module)),
            syn::Type::Paren(paren) => self.type_is_fresh(&paren.elem, module),
            _ => false,
        }
    }

    pub(super) fn type_fully_decodable(
        &self,
        ty: &syn::Type,
        module: &[String],
        visiting: &mut BTreeSet<String>,
    ) -> bool {
        match ty {
            syn::Type::Path(path) if path.qself.is_none() => {
                let Some(last) = path.path.segments.last() else {
                    return false;
                };
                let name = last.ident.to_string();
                if is_primitive(&name) {
                    return true;
                }
                if let Some(key) = self.resolve_source(&path.path, module) {
                    if let Some(info) = self.structs.get(&key) {
                        if !visiting.insert(key.clone()) {
                            return true;
                        }
                        let complete = info.fields.iter().enumerate().all(|(index, field)| {
                            let plan = self.field_plan(field, index, info);
                            plan.kind == FieldKind::Retained
                                && self.type_fully_decodable(&field.ty, &info.module, visiting)
                        });
                        visiting.remove(&key);
                        return complete;
                    }
                    return self
                        .enums
                        .get(&key)
                        .is_some_and(|info| info.unit || info.portable_value);
                }
                matches!(name.as_str(), "Option" | "Vec" | "Box")
                    && one_type_argument(last)
                        .is_some_and(|inner| self.type_fully_decodable(inner, module, visiting))
            }
            syn::Type::Array(array) => self.type_fully_decodable(&array.elem, module, visiting),
            syn::Type::Tuple(tuple) if tuple.elems.len() <= 12 => {
                !tuple.elems.is_empty()
                    && tuple
                        .elems
                        .iter()
                        .all(|element| self.type_fully_decodable(element, module, visiting))
            }
            syn::Type::Paren(paren) => self.type_fully_decodable(&paren.elem, module, visiting),
            _ => false,
        }
    }

    pub(super) fn direct_partial_source(&self, ty: &syn::Type, module: &[String]) -> bool {
        let syn::Type::Path(path) = ty else {
            return false;
        };
        path.qself.is_none()
            && self.resolve_source(&path.path, module).is_some()
            && !self.type_fully_decodable(ty, module, &mut BTreeSet::new())
    }

    pub(super) fn resolve_source(&self, path: &syn::Path, module: &[String]) -> Option<String> {
        let mut segments = path
            .segments
            .iter()
            .map(|segment| segment.ident.to_string())
            .collect::<Vec<_>>();
        if segments.is_empty() {
            return None;
        }
        if let Some(alias) = self
            .aliases
            .get(&module.join("::"))
            .and_then(|aliases| aliases.get(&segments[0]))
        {
            segments = alias
                .iter()
                .cloned()
                .chain(segments.into_iter().skip(1))
                .collect();
        }
        let candidates = path_candidates(&segments, module);
        candidates
            .into_iter()
            .map(|candidate| candidate.join("::"))
            .find(|candidate| {
                self.structs.contains_key(candidate) || self.enums.contains_key(candidate)
            })
    }
}
