use crate::domain::file_entry::{is_archive_extension, FileEntry, FoldersPosition, SortMode};
use rayon::prelude::*;
use std::borrow::Cow;
use std::cmp::Ordering;

/// PERFORMANCE: Converte String para Cow<str> para evitar clones desnecessários
#[inline]
fn to_cow_str(s: &str) -> Cow<'_, str> {
    Cow::Borrowed(s)
}

/// PERFORMANCE: Comparação de nomes com Cow<str> para evitar alocações
#[allow(dead_code)] // Pode ser usado no futuro
#[inline]
fn compare_names_cow(a_name: &str, b_name: &str) -> Ordering {
    natord::compare_ignore_case(a_name, b_name)
}

/// Sorts a slice of FileEntry in place based on the given criteria.
/// Uses Rayon for parallel sorting if the list is large (>5000 items).
///
/// PERFORMANCE: Usa Cow<str> para evitar clones de String em comparações.
/// Mantém a mesma API para compatibilidade retroativa.
pub fn sort_items(
    items: &mut [FileEntry],
    mode: SortMode,
    descending: bool,
    folders_position: FoldersPosition,
) {
    // Helper para verificar se é diretório "verdadeiro" (não arquivo compactado)
    let is_true_dir =
        |item: &FileEntry| -> bool { item.is_dir && !is_archive_extension(&item.name) };

    let compare = |a: &FileEntry, b: &FileEntry| -> Ordering {
        // 1. Lógica de posicionamento de pastas (ZIPs são tratados como arquivos)
        let a_is_dir = is_true_dir(a);
        let b_is_dir = is_true_dir(b);
        if folders_position != FoldersPosition::Mixed && a_is_dir != b_is_dir {
            let folders_come_first = folders_position == FoldersPosition::First;
            return if a_is_dir {
                if folders_come_first {
                    Ordering::Less
                } else {
                    Ordering::Greater
                }
            } else if folders_come_first {
                Ordering::Greater
            } else {
                Ordering::Less
            };
        }

        // 2. Critério principal de ordenação
        // PERFORMANCE: Usa comparação direta sem alocações
        let ordering = match mode {
            SortMode::Name => {
                // PERFORMANCE: Comparação com Cow<str> - evita clones
                let a_name_cow = to_cow_str(&a.name);
                let b_name_cow = to_cow_str(&b.name);
                natord::compare_ignore_case(&a_name_cow, &b_name_cow)
            }
            SortMode::Date => a.modified.cmp(&b.modified),
            SortMode::Size => a.size.cmp(&b.size),
            SortMode::Type => {
                // PERFORMANCE: Compara extensões sem alocação usando OsStr
                let ext_a = a.path.extension().map(|e| e.to_ascii_lowercase());
                let ext_b = b.path.extension().map(|e| e.to_ascii_lowercase());
                match ext_a.cmp(&ext_b) {
                    Ordering::Equal => {
                        // PERFORMANCE: Se extensões forem iguais, compara nomes com Cow
                        let a_name_cow = to_cow_str(&a.name);
                        let b_name_cow = to_cow_str(&b.name);
                        natord::compare_ignore_case(&a_name_cow, &b_name_cow)
                    }
                    other => other,
                }
            }
            SortMode::DriveTotalSpace => {
                // Ordena por espaço total do drive
                let total_a = a.drive_info.as_ref().map(|d| d.total_space).unwrap_or(0);
                let total_b = b.drive_info.as_ref().map(|d| d.total_space).unwrap_or(0);
                total_a.cmp(&total_b)
            }
            SortMode::DriveFreeSpace => {
                // Ordena por espaço livre do drive
                let free_a = a.drive_info.as_ref().map(|d| d.free_space).unwrap_or(0);
                let free_b = b.drive_info.as_ref().map(|d| d.free_space).unwrap_or(0);
                free_a.cmp(&free_b)
            }
        };

        // 3. Aplica direção decrescente
        if descending {
            ordering.reverse()
        } else {
            ordering
        }
    };

    // Limiar adaptativo
    const PARALLEL_THRESHOLD: usize = 5000;

    if items.len() > PARALLEL_THRESHOLD {
        items.par_sort_by(compare);
    } else {
        items.sort_by(compare);
    }
}

/// PERFORMANCE: Filtra items usando Cow<str> para reduzir alocações
pub fn filter_items_cow(items: &[FileEntry], filter: &str) -> Vec<FileEntry> {
    let filter_cow = to_cow_str(filter);
    items
        .iter()
        .filter(|item| {
            let name_cow = to_cow_str(&item.name);
            name_cow.to_lowercase().contains(&filter_cow.to_lowercase())
        })
        .cloned()
        .collect()
}

/// PERFORMANCE: Filtra items com cache de resultados
pub fn filter_items_opt(items: &[FileEntry], filter: &str) -> Vec<FileEntry> {
    if filter.is_empty() {
        return items.to_vec();
    }
    filter_items_cow(items, filter)
}

/// Wrapper para compatibilidade retroativa
pub fn filter_items(items: &[FileEntry], filter: &str) -> Vec<FileEntry> {
    filter_items_opt(items, filter)
}
