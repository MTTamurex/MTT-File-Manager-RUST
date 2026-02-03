use crate::domain::file_entry::SortMode;

pub enum ListViewAction {
    Click(usize),
    DoubleClick(usize),
    SecondaryClick(usize),
    SortChange(SortMode),
    EmptyAreaSecondaryClick,
}
