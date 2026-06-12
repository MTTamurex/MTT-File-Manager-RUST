/// Windows Cloud Files / Sync Root entry shown by Explorer outside normal drives.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CloudRoot {
    pub path: String,
    pub label: String,
    pub icon_resource: Option<String>,
}
