# Aimer rust-analyzer proc-macro server compatibility patch

This is the `proc-macro-srv` crate from `Cottons29/aimer-rust-analyzer`,
`release` revision `03fcb77246f2568adb0e9b2fa60d19c6cc1686f4`. The local copy
adds the `injected_env_var` method required by current `rustc-dev` versions.
All other implementation files are copied from that revision. The matching
MIT and Apache-2.0 license texts are included here.
