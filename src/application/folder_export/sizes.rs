//! Reuse the native scanner for shallow exports; recursive reports aggregate their
//! existing traversal bottom-up instead of rescanning every overlapping subtree.
use super::*;
use std::path::Path;

pub(super) fn needed(options: &Options) -> bool {
    options
        .columns
        .iter()
        .any(|column| column.enabled && column.field == Field::Column(ColumnId::Size))
        || matches!(
            options.sort,
            SortMode::Size | SortMode::Column(ColumnId::Size)
        )
}

pub(super) fn resolve(
    path: &Path,
    traversable: bool,
    options: &Options,
    progress: &Progress,
    skipped: &mut usize,
) -> Result<Option<u64>, ExportError> {
    if let Some(size) = options
        .folder_sizes
        .get(&crate::domain::file_tag::normalize_tag_path_key(path))
    {
        return Ok(Some(*size));
    }
    if !traversable
        || !options.calculate_sizes
        || options.recursive
        || options.contents == Contents::Files
        || !needed(options)
    {
        return Ok(None);
    }
    progress.calculating_sizes.store(true, Ordering::Relaxed);
    let result = crate::infrastructure::windows::folder_size::calculate_folder_size_parallel(
        path,
        &progress.cancelled,
        |_| {},
    );
    progress.calculating_sizes.store(false, Ordering::Relaxed);
    let result = result.ok_or(ExportError::Cancelled)?;
    progress.check()?;
    if result.inaccessible_dir_count > 0 {
        *skipped = skipped.saturating_add(result.inaccessible_dir_count as usize);
        // A partial scan must not be exported as a completed folder size.
        Ok(None)
    } else {
        Ok(Some(result.total_size))
    }
}

pub(super) fn finish(
    rows: &mut Vec<Row>,
    options: &Options,
    progress: &Progress,
) -> Result<(), ExportError> {
    if !options.recursive || !options.calculate_sizes || !needed(options) {
        return Ok(());
    }
    let mut parents = Vec::with_capacity(rows.len());
    let mut ancestors: Vec<usize> = Vec::new();
    let mut children = vec![Vec::new(); rows.len() + 1];
    let root = rows.len();
    for (index, row) in rows.iter().enumerate() {
        progress.check()?;
        while ancestors
            .last()
            .is_some_and(|parent| rows[*parent].depth >= row.depth)
        {
            ancestors.pop();
        }
        let parent = ancestors.last().copied();
        parents.push(parent);
        children[parent.unwrap_or(root)].push(index);
        if row.directory {
            ancestors.push(index);
        }
    }
    let mut totals = vec![0u64; rows.len()];
    for index in (0..rows.len()).rev() {
        progress.check()?;
        // A link may have its own cached display size, but its target must not
        // be counted again in the containing directory's recursive total.
        if rows[index].directory && !rows[index].traversable {
            continue;
        }
        if rows[index].directory
            && rows[index].traversable
            && rows[index].complete
            && rows[index].size.is_none()
        {
            rows[index].size = Some(totals[index]);
        }
        if let Some(parent) = parents[index] {
            if let Some(size) = rows[index].size {
                totals[parent] = totals[parent].saturating_add(size);
            } else if rows[index].traversable {
                rows[parent].complete = false;
            }
        }
    }
    if let Some(column) = options
        .columns
        .iter()
        .filter(|column| column.enabled)
        .position(|column| column.field == Field::Column(ColumnId::Size))
    {
        for row in rows.iter_mut().filter(|row| row.directory) {
            row.cells[column] = row.size.map(|size| size.to_string()).unwrap_or_default();
        }
    }
    if matches!(
        options.sort,
        SortMode::Size | SortMode::Column(ColumnId::Size)
    ) {
        for siblings in &mut children {
            progress.check()?;
            siblings.sort_by(|a, b| compare(&rows[*a], &rows[*b], options));
        }
        let mut pending: Vec<_> = children[root].iter().rev().copied().collect();
        let mut source: Vec<_> = std::mem::take(rows).into_iter().map(Some).collect();
        while let Some(index) = pending.pop() {
            progress.check()?;
            rows.push(
                source[index]
                    .take()
                    .expect("each report row has one parent"),
            );
            pending.extend(children[index].iter().rev().copied());
        }
    }
    Ok(())
}

pub(super) fn compare(a: &Row, b: &Row, options: &Options) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    let folders = match options.folders {
        FoldersPosition::First => b.directory.cmp(&a.directory),
        FoldersPosition::Last => a.directory.cmp(&b.directory),
        FoldersPosition::Mixed => Ordering::Equal,
    };
    folders.then_with(|| match (a.size, b.size) {
        (None, Some(_)) => Ordering::Greater,
        (Some(_), None) => Ordering::Less,
        (a_size, b_size) => {
            let order = a_size
                .cmp(&b_size)
                .then_with(|| natord::compare_ignore_case(&a.name, &b.name));
            if options.descending {
                order.reverse()
            } else {
                order
            }
        }
    })
}
