//! Windows shell operations.

mod context_menu;
mod file_op;
mod shfile_ops;

pub use context_menu::{open_with_shell, show_shell_context_menu, ContextMenuResult};
pub use file_op::{
    copy_item_with_file_op, copy_items_with_file_op, move_item_with_file_op, move_items_with_file_op,
};
pub use shfile_ops::{
    copy_item_with_shell, copy_items_with_shell, delete_item_with_shell, delete_items_permanently_with_shell,
    delete_items_with_shell, move_item_with_shell, move_items_with_shell, rename_item_with_shell,
};
