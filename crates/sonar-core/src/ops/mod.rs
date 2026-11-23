//! File operations. Everything here is synchronous and blocking; callers run
//! it off the UI thread. Long transfers report through [`TransferObserver`].

mod basic;
mod names;
mod transfer;
mod undo;

pub use basic::{create_folder, delete_permanently, new_folder, rename, trash};
pub use names::{available_name, keep_both_name, validate_name};
pub use transfer::{
    Conflict, ConflictChoice, ConflictKind, ItemInfo, Progress, TransferKind, TransferObserver, TransferOutcome,
    run_transfer,
};
pub use undo::UndoAction;
