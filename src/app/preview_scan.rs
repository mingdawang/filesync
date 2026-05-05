use crate::model::config::CompareMethod;
use crate::model::preview::PreviewEntry;

pub(super) fn run_preview_scan(job: crate::model::job::SyncJob) -> Result<Vec<PreviewEntry>, String> {
    use crate::engine::diff::DiffAction;
    use crate::engine::{diff, hash, scanner};

    let globset = scanner::build_globset(&job.exclusions);
    let mut all_entries = Vec::new();

    for pair in &job.folder_pairs {
        if !pair.enabled {
            continue;
        }
        if !pair.source.exists() {
            return Err(crate::messages::source_not_found(
                &pair.source.display().to_string(),
            ));
        }

        let src_scan = scanner::scan_directory(&pair.source, &globset)
            .map_err(|e| crate::messages::scan_source_failed(&e.to_string()))?;
        if !src_scan.issues.is_empty() {
            let first = &src_scan.issues[0];
            return Err(crate::messages::source_scan_issue(
                src_scan.issues.len(),
                &first.message,
            ));
        }

        let dst_scan = if pair.destination.exists() {
            scanner::scan_directory(&pair.destination, &globset)
                .map_err(|e| crate::messages::scan_destination_failed(&e.to_string()))?
        } else {
            scanner::ScanResult::empty()
        };
        if !dst_scan.issues.is_empty() {
            let first = &dst_scan.issues[0];
            return Err(crate::messages::destination_scan_issue(
                dst_scan.issues.len(),
                &first.message,
            ));
        }

        let mut diffs = diff::compute_diff(&pair.source, &pair.destination, &src_scan, &dst_scan);

        if job.compare_method == CompareMethod::Hash {
            for diff in &mut diffs {
                if diff.action == DiffAction::Update {
                    if let (Some(src_hash), Some(dst_hash)) =
                        (hash::hash_file(&diff.source), hash::hash_file(&diff.destination))
                    {
                        if src_hash == dst_hash {
                            diff.action = DiffAction::Skip;
                        }
                    }
                }
            }
        }

        for diff in diffs {
            all_entries.push(PreviewEntry {
                relative_path: diff.relative_path,
                action: diff.action,
                size: diff.size,
                modified: diff.modified,
            });
        }

        for dir in crate::engine::scan_plan::collect_orphan_dirs(&pair.source, &pair.destination) {
            let relative = dir
                .strip_prefix(&pair.destination)
                .map(|r| r.to_path_buf())
                .unwrap_or(dir);
            all_entries.push(PreviewEntry {
                relative_path: relative,
                action: DiffAction::Orphan,
                size: 0,
                modified: std::time::SystemTime::UNIX_EPOCH,
            });
        }
    }

    all_entries.sort_by_key(|entry| match entry.action {
        DiffAction::Create => 0u8,
        DiffAction::Update => 1,
        DiffAction::Skip => 2,
        DiffAction::Orphan => 3,
    });

    Ok(all_entries)
}
