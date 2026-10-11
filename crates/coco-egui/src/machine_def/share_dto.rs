//! `[[drivewire.shares]]` entries: host folders a VM's DriveWire guest
//! services may reach (`coco_core::drivewire::share`).
//!
//! ```toml
//! [[drivewire.shares]]
//! name = "games"
//! path = "/home/me/coco"
//! access = "read-write"
//! ```
//!
//! `access` defaults to `"read-only"`. Relative paths resolve against the
//! machine's artifact directory, like `[media]`. An entry with an empty
//! path is kept for editing but serves nothing.
//!
//! `[drivewire] named_objects = "games"` selects the share that DriveWire
//! named objects (`coco_core::drivewire::NAMED_DRIVE_FIRST`) resolve in. It
//! must name a share entry; without it, every named-object call fails.

use coco_core::drivewire::share::{ShareAccess, ShareSpec, ShareTable};
use serde::{Deserialize, Serialize};

use super::DriveWireDTO;

/// One saved share.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DriveWireShareDTO {
    pub name: String,
    #[serde(default)]
    pub path: String,
    #[serde(default)]
    pub access: ShareAccessDTO,
}

/// `access` key values.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ShareAccessDTO {
    #[default]
    ReadOnly,
    ReadWrite,
}

impl From<ShareAccessDTO> for ShareAccess {
    fn from(access: ShareAccessDTO) -> Self {
        match access {
            ShareAccessDTO::ReadOnly => Self::ReadOnly,
            ShareAccessDTO::ReadWrite => Self::ReadWrite,
        }
    }
}

impl DriveWireDTO {
    /// Checks the share names, including entries without a folder yet, and
    /// that `named_objects` names one of them.
    pub fn validate_shares(&self) -> Result<(), String> {
        coco_core::drivewire::share::validate_share_names(
            self.shares.iter().map(|share| share.name.as_str()),
        )
        .map_err(|error| error.to_string())?;
        match &self.named_objects {
            Some(name) if !self.has_share(name) => Err(format!(
                "DriveWire named objects use share \"{name}\", which is not defined"
            )),
            _ => Ok(()),
        }
    }

    /// Whether a share entry is called `name`, ignoring case.
    pub fn has_share(&self, name: &str) -> bool {
        self.shares
            .iter()
            .any(|share| share.name.eq_ignore_ascii_case(name))
    }

    /// The shares with a folder, resolved for `slug`'s artifact directory.
    pub fn share_table(&self, slug: &str) -> Result<ShareTable, String> {
        let specs = self
            .shares
            .iter()
            .filter(|share| !share.path.trim().is_empty())
            .map(|share| ShareSpec {
                name: share.name.clone(),
                root: super::resolve_media_path(&share.path, slug),
                access: share.access.into(),
            })
            .collect();
        ShareTable::new(specs)
            .map(|table| table.with_named_objects(self.named_objects.clone()))
            .map_err(|error| error.to_string())
    }
}

#[cfg(test)]
#[path = "share_dto_test.rs"]
mod tests;
