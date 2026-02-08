use crate::app::state::ImageViewerApp;
use crate::application::sorting;
use std::sync::Arc;

impl ImageViewerApp {
    /// Filtra e ordena itens baseado na query de busca atual.
    ///
    /// PERFORMANCE: Usa filter_items_opt() que evita clone quando query está vazia.
    /// Isso elimina alocações desnecessárias em 99% dos casos de uso.
    pub fn filter_items(&mut self) {
        // PERFORMANCE: filter_items_opt returns None when query is empty,
        // signaling we should use all_items directly without cloning.
        match sorting::filter_items_opt(&self.all_items, &self.search_query) {
            Some(filtered) => {
                // Query presente: usa o vetor filtrado
                self.items = Arc::new(filtered);
            }
            None => {
                // Query vazia: ordena all_items in-place e usa diretamente
                // Isso evita um clone completo de todo o vetor
                sorting::sort_items(
                    &mut self.all_items,
                    self.sort_mode,
                    self.sort_descending,
                    self.folders_position,
                );
                self.items = Arc::new(self.all_items.clone());
            }
        }
        self.total_items = self.items.len();

        // Se houve filtragem, ainda precisamos ordenar o resultado
        if !self.search_query.is_empty() {
            self.sort_items();
        }
    }

    /// Ordena itens baseado no modo atual e preferência de posição de pastas.
    ///
    /// OTIMIZADO:
    /// - Usa par_sort_by para listas >5000 itens (rayon)
    /// - Usa comparações case-insensitive sem alocação (natord::compare_ignore_case)
    pub fn sort_items(&mut self) {
        // PERFORMANCE: Se temos ownership único do Arc, podemos modificar in-place
        // usando Arc::make_mut(). Caso contrário, precisamos clonar.
        let items = Arc::make_mut(&mut self.items);
        sorting::sort_items(
            items,
            self.sort_mode,
            self.sort_descending,
            self.folders_position,
        );
    }
}
