use crate::domain::file_entry::{is_archive_extension, FileEntry, GroupMode};
use rust_i18n::t;
use std::cmp::Ordering;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use windows::Win32::Foundation::{FILETIME, SYSTEMTIME};
use windows::Win32::System::Time::{FileTimeToSystemTime, SystemTimeToTzSpecificLocalTime};

const UNIX_TO_FILETIME_TICKS: u64 = 116_444_736_000_000_000;
const HUNDRED_NS_PER_SEC: u64 = 10_000_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum NameGroup {
    AToH,
    IToP,
    QToZ,
    Digits,
    Other,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum DateGroup {
    LongAgo,
    EarlierThisYear,
    LastMonth,
    EarlierThisMonth,
    LastWeek,
    EarlierThisWeek,
    Yesterday,
    Today,
    Unspecified,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum SizeGroup {
    Empty,
    Tiny,
    Small,
    Medium,
    Large,
    Huge,
    Gigantic,
    Unspecified,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum TypeGroup {
    Folder,
    NoExtension,
    Extension(String),
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum GroupKey {
    Name(NameGroup),
    Date(DateGroup),
    Type(TypeGroup),
    Size(SizeGroup),
    LocalDrives,
    NetworkDrives,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GroupSection {
    pub key: GroupKey,
    /// Indices into the filtered and sorted `items` snapshot.
    pub item_indices: Arc<[usize]>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GroupProjection {
    pub sections: Vec<GroupSection>,
}

impl GroupProjection {
    pub fn is_grouped(&self) -> bool {
        !self.sections.is_empty()
    }

    pub fn item_count(&self) -> usize {
        self.sections
            .iter()
            .map(|section| section.item_indices.len())
            .sum()
    }
}

pub fn is_grouping_rendered(
    view_mode: crate::domain::file_entry::ViewMode,
    projection: &GroupProjection,
) -> bool {
    view_mode != crate::domain::file_entry::ViewMode::Miller && projection.is_grouped()
}

pub fn build_group_projection(
    items: &[FileEntry],
    mode: GroupMode,
    descending: bool,
) -> GroupProjection {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0);
    build_group_projection_at(items, mode, descending, now)
}

pub fn build_computer_projection(items: &[FileEntry]) -> GroupProjection {
    let mut local = Vec::new();
    let mut network = Vec::new();
    for (index, item) in items.iter().enumerate() {
        let is_remote = item.drive_info.as_ref().is_some_and(|drive| {
            drive.drive_type == crate::infrastructure::windows::DriveType::Remote
        });
        if is_remote {
            network.push(index);
        } else {
            local.push(index);
        }
    }

    let mut sections = Vec::with_capacity(2);
    if !local.is_empty() {
        sections.push(GroupSection {
            key: GroupKey::LocalDrives,
            item_indices: local.into(),
        });
    }
    if !network.is_empty() {
        sections.push(GroupSection {
            key: GroupKey::NetworkDrives,
            item_indices: network.into(),
        });
    }
    GroupProjection { sections }
}

pub fn group_label(key: &GroupKey) -> String {
    match key {
        GroupKey::Name(NameGroup::AToH) => "A - H".to_string(),
        GroupKey::Name(NameGroup::IToP) => "I - P".to_string(),
        GroupKey::Name(NameGroup::QToZ) => "Q - Z".to_string(),
        GroupKey::Name(NameGroup::Digits) => "0 - 9".to_string(),
        GroupKey::Name(NameGroup::Other) => t!("grouping.other").to_string(),
        GroupKey::Date(DateGroup::Today) => t!("grouping.today").to_string(),
        GroupKey::Date(DateGroup::Yesterday) => t!("grouping.yesterday").to_string(),
        GroupKey::Date(DateGroup::EarlierThisWeek) => t!("grouping.earlier_this_week").to_string(),
        GroupKey::Date(DateGroup::LastWeek) => t!("grouping.last_week").to_string(),
        GroupKey::Date(DateGroup::EarlierThisMonth) => {
            t!("grouping.earlier_this_month").to_string()
        }
        GroupKey::Date(DateGroup::LastMonth) => t!("grouping.last_month").to_string(),
        GroupKey::Date(DateGroup::EarlierThisYear) => t!("grouping.earlier_this_year").to_string(),
        GroupKey::Date(DateGroup::LongAgo) => t!("grouping.long_ago").to_string(),
        GroupKey::Date(DateGroup::Unspecified) => t!("grouping.unspecified").to_string(),
        GroupKey::Type(TypeGroup::Folder) => t!("file_types.folder").to_string(),
        GroupKey::Type(TypeGroup::NoExtension) => t!("file_info.file_unknown").to_string(),
        GroupKey::Type(TypeGroup::Extension(extension)) => {
            crate::domain::file_entry::archive_type_label(&format!("file.{extension}"))
                .unwrap_or_else(|| {
                    t!("file_info.file_generic", ext = extension.to_uppercase()).to_string()
                })
        }
        GroupKey::Size(SizeGroup::Empty) => t!("grouping.empty").to_string(),
        GroupKey::Size(SizeGroup::Tiny) => t!("grouping.tiny").to_string(),
        GroupKey::Size(SizeGroup::Small) => t!("grouping.small").to_string(),
        GroupKey::Size(SizeGroup::Medium) => t!("grouping.medium").to_string(),
        GroupKey::Size(SizeGroup::Large) => t!("grouping.large").to_string(),
        GroupKey::Size(SizeGroup::Huge) => t!("grouping.huge").to_string(),
        GroupKey::Size(SizeGroup::Gigantic) => t!("grouping.gigantic").to_string(),
        GroupKey::Size(SizeGroup::Unspecified) => t!("grouping.unspecified").to_string(),
        GroupKey::LocalDrives => t!("sidebar.local_disks").to_string(),
        GroupKey::NetworkDrives => t!("sidebar.network_drives").to_string(),
    }
}

pub fn visible_item_indices(
    projection: &GroupProjection,
    collapsed: Option<&rustc_hash::FxHashSet<GroupKey>>,
    item_count: usize,
) -> Vec<usize> {
    if !projection.is_grouped() {
        return (0..item_count).collect();
    }
    projection
        .sections
        .iter()
        .filter(|section| !collapsed.is_some_and(|groups| groups.contains(&section.key)))
        .flat_map(|section| section.item_indices.iter().copied())
        .filter(|index| *index < item_count)
        .collect()
}

pub fn grid_visual_slots(
    projection: &GroupProjection,
    collapsed: Option<&rustc_hash::FxHashSet<GroupKey>>,
    item_count: usize,
    columns: usize,
) -> Vec<Option<usize>> {
    let columns = columns.max(1);
    if !projection.is_grouped() {
        return (0..item_count).map(Some).collect();
    }
    let mut slots = Vec::new();
    for section in &projection.sections {
        if collapsed.is_some_and(|groups| groups.contains(&section.key)) {
            continue;
        }
        slots.extend(
            section
                .item_indices
                .iter()
                .copied()
                .filter(|index| *index < item_count)
                .map(Some),
        );
        while slots.len() % columns != 0 {
            slots.push(None);
        }
    }
    slots
}

pub fn column_visual_slots(
    projection: &GroupProjection,
    collapsed: Option<&rustc_hash::FxHashSet<GroupKey>>,
    item_count: usize,
    rows_per_column: usize,
) -> Vec<Option<usize>> {
    let rows_per_column = rows_per_column.max(1);
    if !projection.is_grouped() {
        return (0..item_count).map(Some).collect();
    }
    let mut slots = Vec::new();
    for section in &projection.sections {
        if collapsed.is_some_and(|groups| groups.contains(&section.key)) {
            slots.extend(std::iter::repeat_n(None, rows_per_column));
            continue;
        }
        slots.extend(
            section
                .item_indices
                .iter()
                .copied()
                .filter(|index| *index < item_count)
                .map(Some),
        );
        while slots.len() % rows_per_column != 0 {
            slots.push(None);
        }
    }
    slots
}

pub fn resolve_visual_slot(
    slots: &[Option<usize>],
    target: usize,
    previous: Option<usize>,
    block_size: usize,
    advance_past_padding: bool,
) -> Option<usize> {
    if slots.is_empty() {
        return None;
    }
    let target = target.min(slots.len().saturating_sub(1));
    if let Some(index) = slots.get(target).copied().flatten() {
        return Some(index);
    }
    if advance_past_padding {
        if let Some(index) = slots[target.saturating_add(1)..]
            .iter()
            .find_map(|slot| *slot)
        {
            return Some(index);
        }
    }
    let block_size = block_size.max(1);
    let block_start = target / block_size * block_size;
    let block_end = (block_start + block_size).min(slots.len());
    if let Some(index) = slots[block_start..block_end]
        .iter()
        .rev()
        .find_map(|slot| *slot)
    {
        return Some(index);
    }
    if previous.is_some_and(|previous| target < previous) {
        slots[..target].iter().rev().find_map(|slot| *slot)
    } else {
        slots[target.saturating_add(1)..]
            .iter()
            .find_map(|slot| *slot)
            .or_else(|| slots[..target].iter().rev().find_map(|slot| *slot))
    }
}

fn build_group_projection_at(
    items: &[FileEntry],
    mode: GroupMode,
    descending: bool,
    now_unix: u64,
) -> GroupProjection {
    if mode == GroupMode::None || items.is_empty() {
        return GroupProjection::default();
    }

    let today = unix_to_local_date(now_unix);
    let mut positions: HashMap<GroupKey, usize> = HashMap::new();
    let mut sections: Vec<(GroupKey, Vec<usize>)> = Vec::new();

    for (index, item) in items.iter().enumerate() {
        let key = group_key(item, mode, today);
        if let Some(section_index) = positions.get(&key).copied() {
            sections[section_index].1.push(index);
        } else {
            positions.insert(key.clone(), sections.len());
            sections.push((key, vec![index]));
        }
    }

    sections.sort_by(|left, right| {
        let ordering = compare_group_keys(&left.0, &right.0);
        if descending {
            ordering.reverse()
        } else {
            ordering
        }
    });
    GroupProjection {
        sections: sections
            .into_iter()
            .map(|(key, item_indices)| GroupSection {
                key,
                item_indices: item_indices.into(),
            })
            .collect(),
    }
}

fn group_key(item: &FileEntry, mode: GroupMode, today: Option<CivilDate>) -> GroupKey {
    match mode {
        GroupMode::None => unreachable!("ungrouped projections return before classification"),
        GroupMode::Name => GroupKey::Name(classify_name(&item.name)),
        GroupMode::Date => GroupKey::Date(classify_date(item.modified, today)),
        GroupMode::Type => GroupKey::Type(classify_type(item)),
        GroupMode::Size => GroupKey::Size(classify_size(item)),
    }
}

fn classify_name(name: &str) -> NameGroup {
    let Some(first) = name.chars().find(|character| !character.is_whitespace()) else {
        return NameGroup::Other;
    };
    let normalized = normalize_latin_initial(first);
    match normalized {
        'A'..='H' => NameGroup::AToH,
        'I'..='P' => NameGroup::IToP,
        'Q'..='Z' => NameGroup::QToZ,
        '0'..='9' => NameGroup::Digits,
        _ => NameGroup::Other,
    }
}

fn normalize_latin_initial(value: char) -> char {
    match value.to_uppercase().next().unwrap_or(value) {
        'À' | 'Á' | 'Â' | 'Ã' | 'Ä' | 'Å' => 'A',
        'Ç' => 'C',
        'È' | 'É' | 'Ê' | 'Ë' => 'E',
        'Ì' | 'Í' | 'Î' | 'Ï' => 'I',
        'Ñ' => 'N',
        'Ò' | 'Ó' | 'Ô' | 'Õ' | 'Ö' => 'O',
        'Ù' | 'Ú' | 'Û' | 'Ü' => 'U',
        'Ý' | 'Ÿ' => 'Y',
        upper => upper,
    }
}

fn classify_type(item: &FileEntry) -> TypeGroup {
    if is_physical_folder(item) {
        return TypeGroup::Folder;
    }
    if let Some(extension) = crate::domain::file_entry::canonical_archive_extension(&item.name) {
        return TypeGroup::Extension(extension.to_string());
    }
    item.path
        .extension()
        .and_then(|extension| extension.to_str())
        .filter(|extension| !extension.is_empty())
        .map(|extension| TypeGroup::Extension(extension.to_lowercase()))
        .unwrap_or(TypeGroup::NoExtension)
}

fn classify_size(item: &FileEntry) -> SizeGroup {
    if is_physical_folder(item) {
        return SizeGroup::Unspecified;
    }
    match item.size {
        0 => SizeGroup::Empty,
        1..=16_384 => SizeGroup::Tiny,
        16_385..=1_048_576 => SizeGroup::Small,
        1_048_577..=134_217_728 => SizeGroup::Medium,
        134_217_729..=1_073_741_824 => SizeGroup::Large,
        1_073_741_825..=4_294_967_296 => SizeGroup::Huge,
        _ => SizeGroup::Gigantic,
    }
}

#[inline]
fn is_physical_folder(item: &FileEntry) -> bool {
    if !item.is_dir || !is_archive_extension(&item.name) {
        return item.is_dir;
    }
    if item.size > 0 {
        return false;
    }

    // Group projections are rebuilt outside rendering. Probe only the
    // ambiguous archive-like zero-size case; unavailable paths stay archives.
    item.path.metadata().is_ok_and(|metadata| metadata.is_dir())
}

fn classify_date(timestamp: u64, today: Option<CivilDate>) -> DateGroup {
    if timestamp == 0 {
        return DateGroup::Unspecified;
    }
    let (Some(item_date), Some(today)) = (unix_to_local_date(timestamp), today) else {
        return DateGroup::Unspecified;
    };
    classify_civil_date(item_date, today)
}

fn classify_civil_date(item_date: CivilDate, today: CivilDate) -> DateGroup {
    let item_day = item_date.days_since_epoch();
    let today_day = today.days_since_epoch();
    if item_day == today_day {
        return DateGroup::Today;
    }
    if item_day > today_day {
        return DateGroup::Unspecified;
    }
    if item_day == today_day - 1 {
        return DateGroup::Yesterday;
    }

    let week_start = today_day - (today_day + 3).rem_euclid(7);
    if item_day >= week_start {
        return DateGroup::EarlierThisWeek;
    }
    if item_day >= week_start - 7 {
        return DateGroup::LastWeek;
    }
    if item_date.year == today.year && item_date.month == today.month {
        return DateGroup::EarlierThisMonth;
    }
    let (previous_year, previous_month) = if today.month == 1 {
        (today.year - 1, 12)
    } else {
        (today.year, today.month - 1)
    };
    if item_date.year == previous_year && item_date.month == previous_month {
        return DateGroup::LastMonth;
    }
    if item_date.year == today.year {
        DateGroup::EarlierThisYear
    } else {
        DateGroup::LongAgo
    }
}

fn compare_group_keys(left: &GroupKey, right: &GroupKey) -> Ordering {
    match (left, right) {
        (GroupKey::Name(left), GroupKey::Name(right)) => left.cmp(right),
        (GroupKey::Date(left), GroupKey::Date(right)) => left.cmp(right),
        (GroupKey::Size(left), GroupKey::Size(right)) => left.cmp(right),
        (GroupKey::Type(left), GroupKey::Type(right)) => compare_type_groups(left, right),
        (GroupKey::LocalDrives, GroupKey::LocalDrives)
        | (GroupKey::NetworkDrives, GroupKey::NetworkDrives) => Ordering::Equal,
        (GroupKey::LocalDrives, _) => Ordering::Less,
        (_, GroupKey::LocalDrives) => Ordering::Greater,
        (GroupKey::NetworkDrives, _) => Ordering::Less,
        (_, GroupKey::NetworkDrives) => Ordering::Greater,
        _ => Ordering::Equal,
    }
}

fn compare_type_groups(left: &TypeGroup, right: &TypeGroup) -> Ordering {
    match (left, right) {
        (TypeGroup::Folder, TypeGroup::Folder)
        | (TypeGroup::NoExtension, TypeGroup::NoExtension) => Ordering::Equal,
        (TypeGroup::Folder, _) => Ordering::Less,
        (_, TypeGroup::Folder) => Ordering::Greater,
        (TypeGroup::NoExtension, _) => Ordering::Less,
        (_, TypeGroup::NoExtension) => Ordering::Greater,
        (TypeGroup::Extension(left), TypeGroup::Extension(right)) => {
            natord::compare_ignore_case(left, right)
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct CivilDate {
    year: i64,
    month: i64,
    day: i64,
}

impl CivilDate {
    fn days_since_epoch(self) -> i64 {
        days_from_civil(self.year, self.month, self.day)
    }
}

fn unix_to_local_date(timestamp: u64) -> Option<CivilDate> {
    let ticks = timestamp
        .checked_mul(HUNDRED_NS_PER_SEC)?
        .checked_add(UNIX_TO_FILETIME_TICKS)?;
    let file_time = FILETIME {
        dwLowDateTime: ticks as u32,
        dwHighDateTime: (ticks >> 32) as u32,
    };
    let mut utc = SYSTEMTIME::default();
    let mut local = SYSTEMTIME::default();
    unsafe {
        FileTimeToSystemTime(&file_time, &mut utc).ok()?;
        SystemTimeToTzSpecificLocalTime(None, &utc, &mut local).ok()?;
    }
    Some(CivilDate {
        year: i64::from(local.wYear),
        month: i64::from(local.wMonth),
        day: i64::from(local.wDay),
    })
}

// Howard Hinnant's civil-date algorithm.
fn days_from_civil(mut year: i64, month: i64, day: i64) -> i64 {
    year -= i64::from(month <= 2);
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let year_of_era = year - era * 400;
    let month_prime = month + if month > 2 { -3 } else { 9 };
    let day_of_year = (153 * month_prime + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn entry(name: &str, is_dir: bool, size: u64, modified: u64) -> FileEntry {
        FileEntry {
            path: PathBuf::from(name),
            name: name.to_string(),
            is_dir,
            size,
            modified,
            created: None,
            folder_cover: None,
            drive_info: None,
            sync_status: Default::default(),
            is_hidden: false,
            recycle_bin: None,
        }
    }

    #[test]
    fn name_groups_ignore_case_and_common_accents() {
        assert_eq!(classify_name("arquivo.txt"), NameGroup::AToH);
        assert_eq!(classify_name("Écran.png"), NameGroup::AToH);
        assert_eq!(classify_name("Índice.md"), NameGroup::IToP);
        assert_eq!(classify_name("vídeo.mp4"), NameGroup::QToZ);
        assert_eq!(classify_name("42.txt"), NameGroup::Digits);
        assert_eq!(classify_name("_cache"), NameGroup::Other);
    }

    #[test]
    fn size_groups_use_windows_style_boundaries_and_ignore_folder_size() {
        assert_eq!(
            classify_size(&entry("empty", false, 0, 0)),
            SizeGroup::Empty
        );
        assert_eq!(
            classify_size(&entry("tiny", false, 16_384, 0)),
            SizeGroup::Tiny
        );
        assert_eq!(
            classify_size(&entry("small", false, 16_385, 0)),
            SizeGroup::Small
        );
        assert_eq!(
            classify_size(&entry("medium", false, 1_048_577, 0)),
            SizeGroup::Medium
        );
        assert_eq!(
            classify_size(&entry("folder", true, 999, 0)),
            SizeGroup::Unspecified
        );
        assert_eq!(
            classify_size(&entry("archive.zip", true, 999, 0)),
            SizeGroup::Tiny
        );
    }

    #[test]
    fn real_empty_archive_and_archive_named_directory_group_by_physical_kind() {
        let temp = tempfile::tempdir().expect("create temp dir");
        let directory_path = temp.path().join("Fotos.zip");
        std::fs::create_dir(&directory_path).expect("create archive-named directory");
        let directory = FileEntry::from_path(directory_path, true);

        let archive_path = temp.path().join("empty.zip");
        std::fs::write(&archive_path, []).expect("create empty archive file");
        let archive = FileEntry::from_path(archive_path, true);

        assert_eq!(classify_type(&directory), TypeGroup::Folder);
        assert_eq!(classify_size(&directory), SizeGroup::Unspecified);
        assert_eq!(
            classify_type(&archive),
            TypeGroup::Extension("zip".to_string())
        );
        assert_eq!(classify_size(&archive), SizeGroup::Empty);
    }

    #[test]
    fn unavailable_zero_size_archive_stays_an_archive() {
        let archive = entry("missing-empty.zip", true, 0, 0);

        assert_eq!(
            classify_type(&archive),
            TypeGroup::Extension("zip".to_string())
        );
        assert_eq!(classify_size(&archive), SizeGroup::Empty);
    }

    #[test]
    fn navigable_archive_file_groups_as_archive_with_file_size() {
        let archive = entry("Fotos.zip", true, 999, 0);

        assert_eq!(
            classify_type(&archive),
            TypeGroup::Extension("zip".to_string())
        );
        assert_eq!(classify_size(&archive), SizeGroup::Tiny);
    }

    #[test]
    fn type_groups_treat_archives_as_files() {
        assert_eq!(
            classify_type(&entry("folder", true, 0, 0)),
            TypeGroup::Folder
        );
        assert_eq!(
            classify_type(&entry("archive.ZIP", true, 10, 0)),
            TypeGroup::Extension("zip".to_string())
        );
        assert_eq!(
            classify_type(&entry("README", false, 10, 0)),
            TypeGroup::NoExtension
        );
        assert_eq!(
            classify_type(&entry("bundle.tgz", true, 10, 0)),
            TypeGroup::Extension("tar.gz".to_string())
        );
        assert_eq!(
            classify_type(&entry("bundle.tar.gz", true, 10, 0)),
            TypeGroup::Extension("tar.gz".to_string())
        );
    }

    #[test]
    fn projection_preserves_item_order_inside_each_group() {
        let items = vec![
            entry("z.txt", false, 1, 0),
            entry("a.png", false, 1, 0),
            entry("b.txt", false, 1, 0),
        ];
        let projection = build_group_projection_at(&items, GroupMode::Type, false, 0);
        assert_eq!(projection.item_count(), 3);
        assert_eq!(projection.sections[0].item_indices.as_ref(), &[1]);
        assert_eq!(projection.sections[1].item_indices.as_ref(), &[0, 2]);
    }

    #[test]
    fn descending_reverses_sections_not_items() {
        let items = vec![entry("a.txt", false, 1, 0), entry("z.txt", false, 1, 0)];
        let projection = build_group_projection_at(&items, GroupMode::Name, true, 0);
        assert_eq!(projection.sections[0].key, GroupKey::Name(NameGroup::QToZ));
        assert_eq!(projection.sections[1].key, GroupKey::Name(NameGroup::AToH));
    }

    #[test]
    fn collapsed_sections_are_removed_from_visual_navigation_only() {
        let projection = GroupProjection {
            sections: vec![
                GroupSection {
                    key: GroupKey::Name(NameGroup::AToH),
                    item_indices: vec![0, 2].into(),
                },
                GroupSection {
                    key: GroupKey::Name(NameGroup::QToZ),
                    item_indices: vec![1].into(),
                },
            ],
        };
        let mut collapsed = rustc_hash::FxHashSet::default();
        collapsed.insert(GroupKey::Name(NameGroup::AToH));

        assert_eq!(
            visible_item_indices(&projection, Some(&collapsed), 3),
            vec![1]
        );
        assert_eq!(projection.item_count(), 3);
    }

    #[test]
    fn visual_slots_preserve_group_row_and_column_boundaries() {
        let projection = GroupProjection {
            sections: vec![
                GroupSection {
                    key: GroupKey::Name(NameGroup::AToH),
                    item_indices: vec![0, 1].into(),
                },
                GroupSection {
                    key: GroupKey::Name(NameGroup::QToZ),
                    item_indices: vec![2].into(),
                },
            ],
        };
        assert_eq!(
            grid_visual_slots(&projection, None, 3, 3),
            vec![Some(0), Some(1), None, Some(2), None, None]
        );
        assert_eq!(
            column_visual_slots(&projection, None, 3, 3),
            vec![Some(0), Some(1), None, Some(2), None, None]
        );
        assert_eq!(
            resolve_visual_slot(&[Some(0), None, Some(1)], 1, Some(0), 3, false),
            Some(1)
        );
        assert_eq!(
            resolve_visual_slot(&[Some(0), Some(1), None, Some(2)], 2, Some(1), 3, true),
            Some(2)
        );
        assert_eq!(resolve_visual_slot(&[], 0, None, 3, false), None);
    }

    #[test]
    fn date_groups_follow_calendar_boundaries() {
        let today = CivilDate {
            year: 2026,
            month: 8,
            day: 2,
        };
        assert_eq!(
            classify_civil_date(
                CivilDate {
                    year: 2026,
                    month: 8,
                    day: 2
                },
                today
            ),
            DateGroup::Today
        );
        assert_eq!(
            classify_civil_date(
                CivilDate {
                    year: 2026,
                    month: 8,
                    day: 1
                },
                today
            ),
            DateGroup::Yesterday
        );
        assert_eq!(
            classify_civil_date(
                CivilDate {
                    year: 2026,
                    month: 7,
                    day: 30
                },
                today
            ),
            DateGroup::EarlierThisWeek
        );
        assert_eq!(
            classify_civil_date(
                CivilDate {
                    year: 2026,
                    month: 7,
                    day: 10
                },
                today
            ),
            DateGroup::LastMonth
        );
        assert_eq!(
            classify_civil_date(
                CivilDate {
                    year: 2025,
                    month: 12,
                    day: 1
                },
                today
            ),
            DateGroup::LongAgo
        );
        assert_eq!(
            classify_civil_date(
                CivilDate {
                    year: 2026,
                    month: 8,
                    day: 3
                },
                today
            ),
            DateGroup::Unspecified
        );
    }
}
