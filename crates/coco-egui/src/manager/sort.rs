//! Machine-list ordering and its persisted manager preference.

use std::cmp::Ordering;

use chrono::NaiveDate;

use super::{MachineEntry, ManagerApp};
use crate::config::ManagerSort;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum SortKey {
    Created,
    Name,
}

impl SortKey {
    pub(super) const ALL: [Self; 2] = [Self::Created, Self::Name];

    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Created => "Date created",
            Self::Name => "Name",
        }
    }
}

impl ManagerSort {
    pub(super) fn key(self) -> SortKey {
        match self {
            Self::CreatedDesc | Self::CreatedAsc => SortKey::Created,
            Self::NameAsc | Self::NameDesc => SortKey::Name,
        }
    }

    pub(super) fn with_key(self, key: SortKey) -> Self {
        match (key, self.is_ascending()) {
            (SortKey::Created, true) => Self::CreatedAsc,
            (SortKey::Created, false) => Self::CreatedDesc,
            (SortKey::Name, true) => Self::NameAsc,
            (SortKey::Name, false) => Self::NameDesc,
        }
    }

    pub(super) fn toggled(self) -> Self {
        match self {
            Self::CreatedDesc => Self::CreatedAsc,
            Self::CreatedAsc => Self::CreatedDesc,
            Self::NameAsc => Self::NameDesc,
            Self::NameDesc => Self::NameAsc,
        }
    }

    pub(super) fn is_ascending(self) -> bool {
        matches!(self, Self::CreatedAsc | Self::NameAsc)
    }
}

pub(super) fn sort_entries(entries: &mut [MachineEntry], order: ManagerSort) {
    entries.sort_by(|left, right| compare_entries(left, right, order));
}

fn compare_entries(left: &MachineEntry, right: &MachineEntry, order: ManagerSort) -> Ordering {
    match order {
        ManagerSort::CreatedDesc => compare_created(left, right, true),
        ManagerSort::CreatedAsc => compare_created(left, right, false),
        ManagerSort::NameAsc => compare_name(left, right, false),
        ManagerSort::NameDesc => compare_name(left, right, true),
    }
}

fn compare_created(left: &MachineEntry, right: &MachineEntry, descending: bool) -> Ordering {
    let left_date = creation_date(left);
    let right_date = creation_date(right);
    match (left_date, right_date) {
        (Some(left_date), Some(right_date)) => direction(left_date.cmp(&right_date), descending)
            .then_with(|| compare_name(left, right, false)),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => compare_name(left, right, false),
    }
}

fn creation_date(entry: &MachineEntry) -> Option<NaiveDate> {
    let created = entry.def.created.as_deref()?;
    NaiveDate::parse_from_str(created, crate::machine_def::DATE_FORMAT).ok()
}

fn compare_name(left: &MachineEntry, right: &MachineEntry, descending: bool) -> Ordering {
    direction(
        left.def
            .name
            .to_lowercase()
            .cmp(&right.def.name.to_lowercase()),
        descending,
    )
    .then_with(|| left.slug.cmp(&right.slug))
}

fn direction(ordering: Ordering, descending: bool) -> Ordering {
    if descending {
        ordering.reverse()
    } else {
        ordering
    }
}

impl ManagerApp {
    pub(super) fn apply_manager_sort(&mut self, order: ManagerSort) {
        let scroll_to_selection = self.scroll_to_row.is_some();
        let selection = self.selection.snapshot(&self.entries);
        self.manager_sort = order;
        sort_entries(&mut self.entries, order);
        self.selection.restore(&self.entries, &selection);
        if scroll_to_selection {
            self.scroll_to_row = self.selection.single();
        }
    }

    pub(super) fn change_manager_sort(&mut self, order: ManagerSort) {
        self.apply_manager_sort(order);
        self.sort_error = match self.config_path.as_deref() {
            Some(path) => crate::config::save_manager_sort(path, order).err(),
            None => Some(super::NO_CONFIG_DIR.to_string()),
        };
    }
}

#[cfg(test)]
#[path = "sort_test.rs"]
mod tests;
