use super::*;

impl RustdocCrate {
    pub(super) fn select_item(&self, requested_path: Option<&str>) -> anyhow::Result<SelectedItem> {
        let root = self.root_item()?;
        let root_name = root
            .name
            .as_deref()
            .ok_or_else(|| anyhow!("Rustdoc JSON root item has no name"))?;
        let Some(requested_path) = requested_path
            .map(str::trim)
            .filter(|path| !path.is_empty())
        else {
            return Ok(SelectedItem {
                path: root_name.to_owned(),
                kind: "module".to_owned(),
                docs: root.docs.clone(),
                trait_details: None,
            });
        };

        let mut requested = requested_path
            .trim_start_matches("::")
            .split("::")
            .collect::<Vec<_>>();
        if requested
            .first()
            .is_some_and(|segment| *segment == root_name)
        {
            requested.remove(0);
        }
        if requested.iter().any(|segment| segment.is_empty()) {
            bail!("item_path contains an empty path segment");
        }
        if requested.is_empty() {
            return Ok(SelectedItem {
                path: root_name.to_owned(),
                kind: "module".to_owned(),
                docs: root.docs.clone(),
                trait_details: None,
            });
        }

        let mut matches: Vec<(String, String, Option<String>, Option<TraitDetails>)> = Vec::new();
        let mut path = vec![root_name.to_owned()];
        let mut ancestry = std::collections::HashSet::new();
        self.collect_matches(
            &self.root,
            &mut path,
            &requested,
            root.crate_id,
            &mut ancestry,
            &mut matches,
        );
        matches.sort_by(|left, right| left.0.cmp(&right.0).then(left.1.cmp(&right.1)));
        matches.dedup_by(|left, right| left.0 == right.0 && left.1 == right.1 && left.2 == right.2);

        match matches.as_slice() {
            [(path, kind, docs, trait_details)] => Ok(SelectedItem {
                path: path.clone(),
                kind: kind.clone(),
                docs: docs.clone(),
                trait_details: trait_details.clone(),
            }),
            [] => bail!(
                "item '{}' was not found in the public docs for this crate version",
                requested.join("::")
            ),
            many => bail!(
                "item '{}' is ambiguous in this crate version: {}",
                requested.join("::"),
                many.iter()
                    .map(|(path, kind, _, _)| format!("{path} ({kind})"))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        }
    }

    pub(super) fn search_items(
        &self,
        query: &str,
        limit: usize,
    ) -> anyhow::Result<Vec<CrateSearchResult>> {
        let root = self.root_item()?;
        let root_name = root
            .name
            .as_deref()
            .ok_or_else(|| anyhow!("Rustdoc JSON root item has no name"))?;
        let query_lower = query.to_lowercase();
        let mut matches = Vec::new();
        let mut path = vec![root_name.to_owned()];
        let mut ancestry = std::collections::HashSet::new();
        self.collect_search_matches(
            &self.root,
            &mut path,
            root.crate_id,
            &query_lower,
            &mut ancestry,
            &mut matches,
        );
        matches.sort_by(|left, right| {
            left.0
                .cmp(&right.0)
                .then(left.1.item_path.cmp(&right.1.item_path))
                .then(left.1.item_kind.cmp(&right.1.item_kind))
        });
        matches.dedup_by(|left, right| {
            left.1.item_path == right.1.item_path && left.1.item_kind == right.1.item_kind
        });

        Ok(matches
            .into_iter()
            .take(limit)
            .map(|(_, result)| result)
            .collect())
    }

    fn collect_search_matches(
        &self,
        item_id: &str,
        path: &mut Vec<String>,
        local_crate_id: u32,
        query_lower: &str,
        ancestry: &mut std::collections::HashSet<String>,
        matches: &mut Vec<(u8, CrateSearchResult)>,
    ) {
        if !ancestry.insert(item_id.to_owned()) {
            return;
        }

        let Some(item) = self.index.get(item_id) else {
            ancestry.remove(item_id);
            return;
        };
        if item.crate_id == local_crate_id && !is_glob_use(item) {
            let (effective, kind) = self.effective_item(item);
            let name = item_name(item)
                .or_else(|| path.last().map(String::as_str))
                .unwrap_or_default();
            let item_path = path.join("::");
            let name_lower = name.to_lowercase();
            let path_lower = item_path.to_lowercase();
            let docs_lower = effective.docs.as_deref().unwrap_or_default().to_lowercase();
            let name_match = name_lower.contains(query_lower);
            let path_match = path_lower.contains(query_lower);
            let docs_match = docs_lower.contains(query_lower);

            if name_match || path_match || docs_match {
                let mut matched_in = Vec::new();
                if name_match {
                    matched_in.push("name".to_owned());
                }
                if path_match {
                    matched_in.push("path".to_owned());
                }
                if docs_match {
                    matched_in.push("documentation".to_owned());
                }
                let rank = if path_lower == query_lower {
                    0
                } else if name_lower == query_lower {
                    1
                } else if name_lower.starts_with(query_lower) {
                    2
                } else if path_match {
                    3
                } else {
                    4
                };
                matches.push((
                    rank,
                    CrateSearchResult {
                        name: name.to_owned(),
                        item_path,
                        item_kind: kind,
                        matched_in,
                        summary: effective.docs.as_deref().and_then(short_summary),
                    },
                ));
            }
        }

        for child_id in self.child_item_ids(item) {
            let Some(child) = self.index.get(&child_id) else {
                continue;
            };
            if child.crate_id != local_crate_id {
                continue;
            }
            let name = item_name(child).filter(|name| !name.is_empty());
            if let Some(name) = name {
                path.push(name.to_owned());
            }
            self.collect_search_matches(
                &child_id,
                path,
                local_crate_id,
                query_lower,
                ancestry,
                matches,
            );
            if name.is_some() {
                path.pop();
            }
        }
        ancestry.remove(item_id);
    }

    fn collect_matches(
        &self,
        item_id: &str,
        path: &mut Vec<String>,
        requested: &[&str],
        local_crate_id: u32,
        ancestry: &mut std::collections::HashSet<String>,
        matches: &mut Vec<(String, String, Option<String>, Option<TraitDetails>)>,
    ) {
        if !ancestry.insert(item_id.to_owned()) {
            return;
        }

        let Some(item) = self.index.get(item_id) else {
            ancestry.remove(item_id);
            return;
        };
        let relative_path = &path[1..];
        if item.crate_id == local_crate_id
            && !is_glob_use(item)
            && relative_path
                .iter()
                .map(String::as_str)
                .eq(requested.iter().copied())
        {
            let (effective, kind) = self.effective_item(item);
            let trait_details = (kind == "trait").then(|| self.trait_details(effective));
            matches.push((path.join("::"), kind, effective.docs.clone(), trait_details));
        }

        if relative_path.len() > requested.len()
            || !relative_path
                .iter()
                .map(String::as_str)
                .eq(requested[..relative_path.len()].iter().copied())
        {
            ancestry.remove(item_id);
            return;
        }

        for child_id in self.child_item_ids(item) {
            let Some(child) = self.index.get(&child_id) else {
                continue;
            };
            if child.crate_id != local_crate_id {
                continue;
            }
            let name = item_name(child).filter(|name| !name.is_empty());
            if let Some(name) = name {
                path.push(name.to_owned());
            }
            self.collect_matches(
                &child_id,
                path,
                requested,
                local_crate_id,
                ancestry,
                matches,
            );
            if name.is_some() {
                path.pop();
            }
        }
        ancestry.remove(item_id);
    }

    fn child_item_ids(&self, item: &RustdocItem) -> Vec<String> {
        let (item, _) = self.effective_item(item);
        let Some(inner) = item.inner.as_object() else {
            return Vec::new();
        };
        if let Some(module) = inner.get("module") {
            return ids_in(module.get("items"))
                .into_iter()
                .filter(|id| self.index.get(id).is_some_and(public_visibility))
                .collect();
        }
        if let Some(trait_) = inner.get("trait") {
            return ids_in(trait_.get("items"));
        }
        if let Some(enumeration) = inner.get("enum") {
            let mut ids = ids_in(enumeration.get("variants"));
            ids.extend(impl_item_ids(self, enumeration.get("impls")));
            return ids;
        }
        if let Some(structure) = inner.get("struct") {
            let mut ids = Vec::new();
            if let Some(kind) = structure.get("kind") {
                if let Some(plain) = kind.get("plain") {
                    ids.extend(
                        ids_in(plain.get("fields"))
                            .into_iter()
                            .filter(|id| self.index.get(id).is_some_and(public_visibility)),
                    );
                }
                if let Some(tuple) = kind.get("tuple") {
                    ids.extend(ids_in(Some(tuple)));
                }
            }
            ids.extend(impl_item_ids(self, structure.get("impls")));
            return ids;
        }
        if let Some(union) = inner.get("union") {
            let mut ids = ids_in(union.get("fields"))
                .into_iter()
                .filter(|id| self.index.get(id).is_some_and(public_visibility))
                .collect::<Vec<_>>();
            ids.extend(impl_item_ids(self, union.get("impls")));
            return ids;
        }
        if let Some(variant) = inner.get("variant") {
            return ids_in(variant.get("fields"));
        }
        if let Some(implementation) = inner.get("impl") {
            return ids_in(implementation.get("items"));
        }
        if let Some(primitive) = inner.get("primitive") {
            return impl_item_ids(self, primitive.get("impls"));
        }
        if let Some(use_item) = inner.get("use") {
            return id_in(use_item.get("id")).into_iter().collect();
        }
        Vec::new()
    }

    fn effective_item<'a>(&'a self, mut item: &'a RustdocItem) -> (&'a RustdocItem, String) {
        let mut visited = std::collections::HashSet::new();
        while let Some(id) = item
            .inner
            .get("use")
            .and_then(|value| id_in(value.get("id")))
        {
            if !visited.insert(id.clone()) {
                break;
            }
            let Some(target) = self.index.get(&id) else {
                break;
            };
            item = target;
        }
        let kind = item
            .inner
            .as_object()
            .and_then(|inner| inner.keys().next())
            .cloned()
            .unwrap_or_else(|| "item".to_owned());
        (item, kind)
    }

    fn trait_details(&self, item: &RustdocItem) -> TraitDetails {
        let trait_data = item.inner.get("trait").unwrap_or(&serde_json::Value::Null);
        let name = item.name.as_deref().unwrap_or("Trait");
        let signature = format_trait_signature(name, trait_data);
        let methods = trait_data
            .get("items")
            .map(|items| ids_in(Some(items)))
            .unwrap_or_default()
            .into_iter()
            .filter_map(|id| self.index.get(&id))
            .filter_map(|method| {
                let name = method.name.as_deref()?.to_owned();
                let function = method.inner.get("function")?;
                let required = !function
                    .get("has_body")
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(false);
                Some(TraitMethodDocumentation {
                    name: name.clone(),
                    required,
                    signature: format_method_signature(&name, function),
                    documentation: method.docs.clone(),
                })
            })
            .collect::<Vec<_>>();
        let definition = format_trait_definition(&signature, &methods);

        TraitDetails {
            signature,
            definition,
            methods,
        }
    }

    pub(super) fn root_item(&self) -> anyhow::Result<&RustdocItem> {
        self.index
            .get(&self.root)
            .ok_or_else(|| anyhow!("Rustdoc JSON root item is missing from its index"))
    }
}

fn item_name(item: &RustdocItem) -> Option<&str> {
    if is_glob_use(item) {
        return None;
    }
    item.name
        .as_deref()
        .or_else(|| item.inner.get("use")?.get("name")?.as_str())
}

fn is_glob_use(item: &RustdocItem) -> bool {
    item.inner
        .get("use")
        .and_then(|value| value.get("is_glob"))
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false)
}
