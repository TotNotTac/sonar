//! Core filesystem model for Sonar.
//!
//! This crate is UI-agnostic: it knows how to read directories, describe
//! entries, build breadcrumbs and enumerate "places", but nothing about Tauri.
//! Everything OS-specific lives behind [`platform`].

pub mod breadcrumb;
pub mod clipboard;
pub mod entry;
pub mod error;
pub mod filetype;
pub mod listing;
pub mod ops;
pub mod paths;
pub mod places;
pub mod platform;
pub mod sort;
pub mod watch;

pub use breadcrumb::{breadcrumbs, Crumb};
pub use entry::{Entry, EntryKind};
pub use error::{CoreError, Result};
pub use listing::{child_count, list_dir, ListOptions, Listing};
pub use paths::resolve_user_path;
pub use places::{places, Place, PlaceSection};
pub use platform::open_path;
pub use sort::{SortKey, SortOrder, SortSpec};
pub use watch::DirWatcher;
