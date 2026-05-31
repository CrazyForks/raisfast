#[cfg(feature = "export-types")]
mod private {
    use std::collections::HashSet;
    use ts_rs::TS;

    pub struct ExportType {
        pub name_fn: fn(&ts_rs::Config) -> String,
        pub decl_fn: fn(&ts_rs::Config) -> String,
    }

    inventory::collect!(ExportType);

    impl ExportType {
        pub const fn new<T: TS>() -> Self {
            Self {
                name_fn: <T as TS>::name,
                decl_fn: <T as TS>::decl,
            }
        }
    }

    pub fn collect_all() -> Vec<(String, String)> {
        let cfg = ts_rs::Config::default();
        let mut seen = HashSet::new();
        let mut decls: Vec<(String, String)> = Vec::new();
        for et in inventory::iter::<ExportType> {
            let name = (et.name_fn)(&cfg);
            if !seen.contains(&name) {
                seen.insert(name.clone());
                let decl = (et.decl_fn)(&cfg);
                decls.push((name, decl));
            }
        }
        decls.sort_by(|a, b| a.0.cmp(&b.0));
        decls
    }
}

#[cfg(feature = "export-types")]
pub use private::{ExportType, collect_all};
