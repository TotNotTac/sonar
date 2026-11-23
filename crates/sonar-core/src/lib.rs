//! Core filesystem model for Sonar.
//!
//! This crate is UI-agnostic: it knows how to read directories, describe
//! entries, build breadcrumbs and enumerate "places", but nothing about Tauri.
//! Everything OS-specific lives behind [`platform`].

pub mod archive;
pub mod breadcrumb;
pub mod clipboard;
pub mod desktop;
pub mod entry;
pub mod error;
pub mod filetype;
pub mod listing;
pub mod mimeglob;
pub mod ops;
pub mod paths;
pub mod places;
pub mod platform;
pub mod properties;
pub mod search;
pub mod sort;
pub mod thumbnail;
pub mod uipath;
pub mod volumes;
pub mod watch;

pub use breadcrumb::{Crumb, breadcrumbs};
pub use entry::{Entry, EntryKind};
pub use error::{CoreError, Result};
pub use listing::{ListOptions, Listing, child_count, list_dir};
pub use paths::resolve_user_path;
pub use places::{Place, PlaceSection, places};
pub use platform::open_path;
pub use sort::{SortKey, SortOrder, SortSpec};
pub use watch::DirWatcher;
