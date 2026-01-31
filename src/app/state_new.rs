use crate::app::cache_state::CacheState;
use crate::app::navigation_state::NavigationState;
use crate::app::ui_state::UIState;
use crate::app::worker_state::WorkerState;
use crate::application::sorting::{SortMode, FoldersPosition};
use eframe::egui;

/// Estado da aplicação refatorado com módulos separados
/// 
/// Esta estrutura substitui o ImageViewerApp monolítico original,
/// dividindo a responsabilidade em módulos menores e mais manuteníveis.
pub struct ImageViewerApp {
    /// Estado de navegação
    pub navigation: NavigationState,
    
    /// Estado de cache
    pub cache: CacheState,
    
    /// Estado de UI
    pub ui: UIState,
    
    /// Estado de workers
    pub workers: WorkerState,
    
    /// Configurações de ordenação
    pub sort_mode: SortMode,
    pub sort_descending: bool,
    pub folders_position: FoldersPosition,
    
    /// Gerenciadores de UI (serão adicionados quando os módulos forem criados)
    // pub clipboard_manager: ClipboardManager,
    // pub context_menu_state: ContextMenuState,
    // pub icon_loader: IconLoader,
    // pub svg_icon_manager: SvgIconManager,
    
    /// Janela principal (Windows)
    pub main_window: Option<egui::os::windows::WindowHandle>,
    
    /// Última entrada do usuário
    pub last_input: LastInput,
    
    /// Flag para indicar se a aplicação está rodando
    pub running: bool,
}

/// Tipo de última entrada do usuário
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LastInput {
    Mouse,
    Keyboard,
    Touch,
}

impl ImageViewerApp {
    /// Cria uma nova instância da aplicação
    pub fn new(ui_ctx: egui::Context) -> Self {
        Self {
            navigation: NavigationState::default(),
            cache: CacheState::new(),
            ui: UIState::new(ui_ctx.clone()),
            workers: WorkerState::new(),
            sort_mode: SortMode::Name,
            sort_descending: false,
            folders_position: FoldersPosition::First,
            // clipboard_manager: ClipboardManager::new(),
            // context_menu_state: ContextMenuState::new(),
            // icon_loader: IconLoader::new(),
            // svg_icon_manager: SvgIconManager::new(),
            main_window: None,
            last_input: LastInput::Mouse,
            running: true,
        }
    }
    
    /// Limpa o estado da aplicação
    pub fn clear_state(&mut self) {
        self.navigation.current_path.clear();
        self.navigation.path_input.clear();
        self.ui.clear();
        self.workers.clear();
        self.cache.clear_all();
        // self.clipboard_manager.clear();
        // self.context_menu_state.hide();
    }
    
    /// Atualiza o estado da aplicação
    pub fn update(&mut self, ctx: &egui::Context) {
        // Processar mensagens dos workers
        self.process_worker_messages();
        
        // Atualizar UI
        self.ui.ui_ctx.request_repaint();
        
        // Limpar mensagens antigas
        self.clear_old_messages();
    }
    
    /// Processa mensagens dos workers
    fn process_worker_messages(&mut self) {
        // Processar thumbnails
        while let Ok(thumbnail) = self.workers.image_receiver.try_recv() {
            // Processar thumbnail recebido
            self.process_thumbnail(thumbnail);
        }
        
        // Processar file entries
        while let Ok((request_id, entries)) = self.workers.file_entry_receiver.try_recv() {
            // Processar entradas de arquivo recebidas
            self.process_file_entries(request_id, entries);
        }
        
        // Processar rebuild results
        while let Ok(result) = self.workers.items_rebuild_receiver.try_recv() {
            // Processar resultado de rebuild
            self.process_rebuild_result(result);
        }
    }
    
    /// Processa um thumbnail recebido
    fn process_thumbnail(&mut self, _thumbnail: crate::domain::thumbnail::ThumbnailData) {
        // Implementação será adicionada quando o sistema de thumbnails estiver completo
    }
    
    /// Processa entradas de arquivo recebidas
    fn process_file_entries(&mut self, _request_id: usize, _entries: Vec<crate::domain::file_entry::FileEntry>) {
        // Implementação será adicionada quando o sistema de carregamento estiver completo
    }
    
    /// Processa resultado de rebuild
    fn process_rebuild_result(&mut self, _result: crate::app::state::ItemsRebuildResult) {
        // Implementação será adicionada quando o sistema de rebuild estiver completo
    }
    
    /// Limpa mensagens antigas
    fn clear_old_messages(&mut self) {
        if let Some(timeout) = self.ui.message_timeout {
            if timeout.elapsed().as_secs() > 5 {
                self.ui.error_message = None;
                self.ui.success_message = None;
                self.ui.message_timeout = None;
            }
        }
    }
}