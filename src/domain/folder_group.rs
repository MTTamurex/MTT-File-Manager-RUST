/// A named group of workspace tab paths shown in Quick Access.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FolderGroup {
    pub id: i64,
    pub name: String,
    pub position: i64,
    pub paths: Vec<String>,
}
