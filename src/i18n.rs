//! Interface language catalog: English and Português (Brasil).
//!
//! Views translate through [`t`]; nothing user-visible should hardcode
//! English (or any language) outside this table. Placeholders use
//! `{name}`-style tokens replaced by the caller, so translators can
//! reorder them.

use serde::{Deserialize, Serialize};

/// Interface language chosen in Settings.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Language {
    #[default]
    English,
    Portugues,
}

impl Language {
    pub const ALL: [Language; 2] = [Self::English, Self::Portugues];

    /// Label shown in the language picker.
    pub fn label(self) -> &'static str {
        match self {
            Self::English => "English",
            Self::Portugues => "Português (Brasil)",
        }
    }

    /// Best-effort guess from `LANG`/`LC_ALL`, defaulting to English.
    pub fn detect() -> Self {
        for key in ["LC_ALL", "LANG", "LANGUAGE"] {
            if let Ok(value) = std::env::var(key) {
                let lower = value.to_ascii_lowercase();
                if lower.starts_with("pt") {
                    return Self::Portugues;
                }
            }
        }
        Self::English
    }
}

/// Translates `key` for `lang`, falling back to English for unknown keys.
pub fn t(lang: Language, key: &str) -> &str {
    lookup(lang, key)
        .or_else(|| lookup(Language::English, key))
        .unwrap_or(key)
}

fn lookup(lang: Language, key: &str) -> Option<&'static str> {
    Some(match (lang, key) {
        // --- language ---------------------------------------------------
        (Language::Portugues, "language.name") => "Português (Brasil)",
        (_, "language.name") => "English",
        // --- people -----------------------------------------------------
        (Language::Portugues, "person.you") => "Você",
        (_, "person.you") => "You",
        (Language::Portugues, "person.unsaved") => "Nome não salvo",
        (_, "person.unsaved") => "Unsaved name",
        // --- settings ---------------------------------------------------
        (Language::Portugues, "settings.title") => "Configurações",
        (_, "settings.title") => "Settings",
        (Language::Portugues, "settings.back") => "Voltar (Esc)",
        (_, "settings.back") => "Back (Esc)",
        (Language::Portugues, "settings.section.appearance") => "Aparência",
        (_, "settings.section.appearance") => "Appearance",
        (Language::Portugues, "settings.theme") => "Tema",
        (_, "settings.theme") => "Theme",
        (Language::Portugues, "settings.theme.follow_system") => "Seguir o sistema",
        (_, "settings.theme.follow_system") => "Follow system",
        (Language::Portugues, "settings.theme.dark") => "Escuro",
        (_, "settings.theme.dark") => "Dark",
        (Language::Portugues, "settings.theme.light") => "Claro",
        (_, "settings.theme.light") => "Light",
        (Language::Portugues, "settings.theme.omarchy") => {
            "Seguir o sistema usa as suas cores do Omarchy."
        }
        (_, "settings.theme.omarchy") => "Follow system uses your Omarchy colours.",
        (Language::Portugues, "settings.theme.system") => {
            "Seguir o sistema usa a aparência clara ou escura da sua área de trabalho."
        }
        (_, "settings.theme.system") => {
            "Follow system uses your desktop's light or dark appearance."
        }
        (Language::Portugues, "settings.open_themes") => "Abrir pasta de temas",
        (_, "settings.open_themes") => "Open themes folder",
        (Language::Portugues, "settings.zoom") => "Zoom",
        (_, "settings.zoom") => "Zoom",
        (Language::Portugues, "settings.zoom.hint") => {
            "Você também pode usar Ctrl+mais e Ctrl+menos."
        }
        (_, "settings.zoom.hint") => "You can also use Ctrl+plus and Ctrl+minus.",
        (Language::Portugues, "settings.zoom.in") => "Aumentar",
        (_, "settings.zoom.in") => "Larger",
        (Language::Portugues, "settings.zoom.out") => "Diminuir",
        (_, "settings.zoom.out") => "Smaller",
        (Language::Portugues, "settings.language") => "Idioma",
        (_, "settings.language") => "Language",
        (Language::Portugues, "settings.language.hint") => {
            "Idioma da interface. Aplica-se na hora, sem religar."
        }
        (_, "settings.language.hint") => {
            "Interface language. Applies immediately, no relink needed."
        }
        (Language::Portugues, "settings.section.chats") => "Conversas",
        (_, "settings.section.chats") => "Chats",
        (Language::Portugues, "settings.enter_sends") => "Enter envia",
        (_, "settings.enter_sends") => "Enter sends",
        (Language::Portugues, "settings.enter_sends.hint") => {
            "Quando desligado, Enter quebra linha e Ctrl+Enter envia."
        }
        (_, "settings.enter_sends.hint") => "When off, Enter adds a line and Ctrl+Enter sends.",
        (Language::Portugues, "settings.read_receipts") => "Enviar confirmações de leitura",
        (_, "settings.read_receipts") => "Send read receipts",
        (Language::Portugues, "settings.read_receipts.off") => {
            "As confirmações de leitura estão desativadas na sua conta do WhatsApp. Conversas diretas não as enviarão. Com este interruptor ligado, grupos ainda enviam. A sincronia de leitura entre os seus aparelhos funciona de qualquer forma."
        }
        (_, "settings.read_receipts.off") => {
            "Read receipts are disabled for your WhatsApp account. Direct chats will not send them. When this switch is on, groups still do. Read state syncs between your devices either way."
        }
        (Language::Portugues, "settings.read_receipts.on") => {
            "Permite que as pessoas vejam quando você lê mensagens ou ouve mensagens de voz. A sua configuração de privacidade do WhatsApp continua valendo. A sincronia de leitura entre os seus aparelhos funciona de qualquer forma."
        }
        (_, "settings.read_receipts.on") => {
            "Let people see when you read messages or play voice messages. Your WhatsApp privacy setting still applies. Read state syncs between your devices either way."
        }
        (Language::Portugues, "settings.typing") => "Mostrar quando você está digitando",
        (_, "settings.typing") => "Show when you are typing",
        (Language::Portugues, "settings.auto_download") => "Baixar anexos automaticamente",
        (_, "settings.auto_download") => "Download attachments automatically",
        (Language::Portugues, "settings.auto_download.hint") => {
            "Baixa fotos, vídeos, mensagens de voz e documentos de até 64 MB ao entrar em vista. Quando desligado, clique no arquivo para baixar."
        }
        (_, "settings.auto_download.hint") => {
            "Download pictures, videos, voice messages, and documents up to 64 MB when they enter view. When off, click a file to download it."
        }
        (Language::Portugues, "settings.sender_pictures") => {
            "Mostrar fotos do remetente em todas as conversas"
        }
        (_, "settings.sender_pictures") => "Show sender pictures in every chat",
        (Language::Portugues, "settings.sender_pictures.hint") => {
            "O WhatsApp as mostra só em grupos."
        }
        (_, "settings.sender_pictures.hint") => "WhatsApp shows them in groups only.",
        (Language::Portugues, "settings.names_from_contacts") => "Nomes da sua agenda",
        (_, "settings.names_from_contacts") => "Names from your address book",
        (Language::Portugues, "settings.names_from_contacts.hint") => {
            "Prefere os nomes salvos nos contatos. Quando desligado, prefere os nomes públicos do perfil do WhatsApp. Vale para o aplicativo todo."
        }
        (_, "settings.names_from_contacts.hint") => {
            "Prefer saved contact names. When off, prefer public WhatsApp profile names. This applies throughout the app."
        }
        (Language::Portugues, "settings.save_to_phone") => "Salvar contatos na agenda do telefone",
        (_, "settings.save_to_phone") => "Save contacts to the phone's address book",
        (Language::Portugues, "settings.save_to_phone.hint") => {
            "Também adiciona os contatos salvos aqui à agenda do seu telefone. Quando desligado, eles continuam como contatos do WhatsApp. Os nomes sincronizam com os aparelhos vinculados de qualquer forma."
        }
        (_, "settings.save_to_phone.hint") => {
            "Also add contacts saved here to your phone's address book. When off, they remain WhatsApp contacts. Names sync to linked devices either way."
        }
        (Language::Portugues, "settings.shortcut_hints") => "Mostrar dicas de atalho",
        (_, "settings.shortcut_hints") => "Show shortcut hints",
        (Language::Portugues, "settings.section.window") => "Janela",
        (_, "settings.section.window") => "Window",
        (Language::Portugues, "settings.keep_running") => "Continuar rodando ao fechar a janela",
        (_, "settings.keep_running") => "Keep running when the window closes",
        (Language::Portugues, "settings.keep_running.hint") => {
            "Mantém o ZapFast vinculado na bandeja do sistema. Saia pelo menu da bandeja ou com Ctrl+Q."
        }
        (_, "settings.keep_running.hint") => {
            "Keep ZapFast linked in the system tray. Quit from the tray menu or with Ctrl+Q."
        }
        (Language::Portugues, "settings.notifications") => "Notificar sobre novas mensagens",
        (_, "settings.notifications") => "Notify about new messages",
        (Language::Portugues, "settings.notifications.hint") => {
            "Mostra notificações na área de trabalho quando a janela está oculta, em segundo plano ou mostrando outra conversa. Conversas silenciadas não notificam."
        }
        (_, "settings.notifications.hint") => {
            "Show desktop notifications when the window is hidden, in the background, or showing another chat. Muted chats do not notify you."
        }
        (Language::Portugues, "settings.auto_update") => "Baixar atualizações automaticamente",
        (_, "settings.auto_update") => "Download updates automatically",
        (Language::Portugues, "settings.auto_update.hint") => {
            "Baixa e verifica novos lançamentos em segundo plano. Você escolhe quando reiniciar. Pacotes nativos e Flatpak atualizam pelo gerenciador de pacotes."
        }
        (_, "settings.auto_update.hint") => {
            "Download and verify new releases in the background. You choose when to restart. Native packages and Flatpak update through their package manager."
        }
        (Language::Portugues, "settings.check_updates") => "Verificar atualizações",
        (_, "settings.check_updates") => "Check for updates",
        (Language::Portugues, "settings.check_updates.hint") => {
            "Pergunta ao GitHub uma vez por dia se existe um lançamento mais novo do ZapFast. O pedido identifica só o ZapFast e a sua versão."
        }
        (_, "settings.check_updates.hint") => {
            "Ask GitHub once a day whether a newer ZapFast release exists. The request identifies only ZapFast and its version."
        }
        (Language::Portugues, "settings.giphy") => "Chave da API do GIPHY",
        (_, "settings.giphy") => "GIPHY API key",
        (Language::Portugues, "settings.giphy.builtin") => {
            "Usada na busca de GIFs. Esta compilação inclui uma chave. Informe uma chave de developers.giphy.com para substituí-la."
        }
        (_, "settings.giphy.builtin") => {
            "Used for GIF search. This build includes a key. Enter a key from developers.giphy.com to replace it."
        }
        (Language::Portugues, "settings.giphy.missing") => {
            "Necessária para a busca de GIFs. Obtenha uma chave grátis em developers.giphy.com."
        }
        (_, "settings.giphy.missing") => {
            "Required for GIF search. Get a free key from developers.giphy.com."
        }
        (Language::Portugues, "settings.section.account") => "Conta",
        (_, "settings.section.account") => "Account",
        (Language::Portugues, "settings.linked_device") => "Aparelho vinculado",
        (_, "settings.linked_device") => "Linked device",
        (Language::Portugues, "settings.unlink") => "Desvincular este computador",
        (_, "settings.unlink") => "Unlink this computer",
        (Language::Portugues, "settings.section.files") => "Arquivos",
        (_, "settings.section.files") => "Files",
        (Language::Portugues, "settings.archive") => "Arquivo de mensagens",
        (_, "settings.archive") => "Message archive",
        (Language::Portugues, "settings.open_folder") => "Abrir pasta",
        (_, "settings.open_folder") => "Open folder",
        (Language::Portugues, "settings.media") => "Anexos baixados",
        (_, "settings.media") => "Downloaded attachments",
        (Language::Portugues, "settings.log") => "Registro desta execução",
        (_, "settings.log") => "Log of this run",
        (Language::Portugues, "settings.open") => "Abrir",
        (_, "settings.open") => "Open",
        (Language::Portugues, "settings.section.about") => "Sobre",
        (_, "settings.section.about") => "About",
        (Language::Portugues, "settings.about.hint") => {
            "Um cliente nativo de WhatsApp feito com Rust, egui e whatsapp-rust."
        }
        (_, "settings.about.hint") => {
            "A native WhatsApp client built with Rust, egui, and whatsapp-rust."
        }
        (Language::Portugues, "settings.about") => "Sobre",
        (_, "settings.about") => "About",
        (Language::Portugues, "settings.shortcuts") => "Atalhos",
        (_, "settings.shortcuts") => "Shortcuts",
        // --- chat list --------------------------------------------------
        (Language::Portugues, "chats.title") => "Conversas",
        (_, "chats.title") => "Chats",
        (Language::Portugues, "chats.archived") => "Arquivadas",
        (_, "chats.archived") => "Archived",
        (Language::Portugues, "chats.back") => "Voltar às conversas",
        (_, "chats.back") => "Back to chats",
        (Language::Portugues, "chats.you") => "Você",
        (_, "chats.you") => "You",
        (Language::Portugues, "chats.settings") => "Configurações (Ctrl+,)",
        (_, "chats.settings") => "Settings (Ctrl+,)",
        (Language::Portugues, "chats.new_contact") => "Novo contato",
        (_, "chats.new_contact") => "New contact",
        (Language::Portugues, "chats.new_contact_mac") => "Novo contato (⌘N)",
        (_, "chats.new_contact_mac") => "New contact (⌘N)",
        (Language::Portugues, "chats.hide") => "Ocultar a lista de conversas (Ctrl+B)",
        (_, "chats.hide") => "Hide the chat list (Ctrl+B)",
        (Language::Portugues, "chats.hide_mac") => "Ocultar a lista de conversas (⌘B)",
        (_, "chats.hide_mac") => "Hide the chat list (⌘B)",
        (Language::Portugues, "chats.search") => "Pesquisar",
        (_, "chats.search") => "Search",
        (Language::Portugues, "filter.all") => "Todas",
        (_, "filter.all") => "All",
        (Language::Portugues, "filter.unread") => "Não lidas",
        (_, "filter.unread") => "Unread",
        (Language::Portugues, "filter.private") => "Particulares",
        (_, "filter.private") => "Private",
        (Language::Portugues, "filter.groups") => "Grupos",
        (_, "filter.groups") => "Groups",
        (Language::Portugues, "chats.empty.archived.title") => "Nada arquivado",
        (_, "chats.empty.archived.title") => "Nothing archived",
        (Language::Portugues, "chats.empty.archived.body") => {
            "As conversas arquivadas aparecem aqui."
        }
        (_, "chats.empty.archived.body") => "Archived chats appear here.",
        (Language::Portugues, "chats.empty.unread") => "Nenhuma conversa não lida",
        (_, "chats.empty.unread") => "No unread chats",
        (Language::Portugues, "chats.empty.private") => "Nenhuma conversa particular",
        (_, "chats.empty.private") => "No private chats",
        (Language::Portugues, "chats.empty.groups") => "Nenhum grupo",
        (_, "chats.empty.groups") => "No groups",
        (Language::Portugues, "chats.empty.filtered") => "Escolha Todas para ver cada conversa.",
        (_, "chats.empty.filtered") => "Choose All to see every chat.",
        (Language::Portugues, "chats.empty.loading.title") => "Carregando suas conversas",
        (_, "chats.empty.loading.title") => "Loading your chats",
        (Language::Portugues, "chats.empty.loading.body") => {
            "Recebendo o histórico do seu telefone."
        }
        (_, "chats.empty.loading.body") => "Receiving history from your phone.",
        (Language::Portugues, "chats.empty.title") => "Nenhuma conversa ainda",
        (_, "chats.empty.title") => "No chats yet",
        (Language::Portugues, "chats.empty.body") => {
            "Novas conversas aparecem aqui. Você pode começar uma pelo seu telefone."
        }
        (_, "chats.empty.body") => "New chats appear here. You can start one from your phone.",
        (Language::Portugues, "chats.no_results.title") => "Sem resultados",
        (_, "chats.no_results.title") => "No results",
        (Language::Portugues, "chats.no_results.body") => {
            "Tente outro nome, número ou texto de mensagem."
        }
        (_, "chats.no_results.body") => "Try another name, number, or message text.",
        (Language::Portugues, "search.chats") => "Conversas",
        (_, "search.chats") => "Chats",
        (Language::Portugues, "search.messages") => "Mensagens",
        (_, "search.messages") => "Messages",
        (Language::Portugues, "search.contacts") => "Contatos",
        (_, "search.contacts") => "Contacts",
        (Language::Portugues, "menu.mark_read") => "Marcar como lida",
        (_, "menu.mark_read") => "Mark as read",
        (Language::Portugues, "menu.unpin") => "Desafixar",
        (_, "menu.unpin") => "Unpin",
        (Language::Portugues, "menu.pin") => "Fixar no topo",
        (_, "menu.pin") => "Pin to top",
        (Language::Portugues, "menu.unarchive") => "Desarquivar",
        (_, "menu.unarchive") => "Unarchive",
        (Language::Portugues, "menu.archive") => "Arquivar",
        (_, "menu.archive") => "Archive",
        (Language::Portugues, "menu.unmute") => "Ativar som",
        (_, "menu.unmute") => "Unmute",
        (Language::Portugues, "menu.mute.8h") => "Silenciar por 8 horas",
        (_, "menu.mute.8h") => "Mute for 8 hours",
        (Language::Portugues, "menu.mute.week") => "Silenciar por uma semana",
        (_, "menu.mute.week") => "Mute for a week",
        (Language::Portugues, "menu.mute.forever") => "Silenciar para sempre",
        (_, "menu.mute.forever") => "Mute indefinitely",
        (Language::Portugues, "menu.copy_number") => "Copiar número",
        (_, "menu.copy_number") => "Copy number",
        (Language::Portugues, "menu.info") => "Dados",
        (_, "menu.info") => "Info",
        (Language::Portugues, "menu.reply") => "Responder",
        (_, "menu.reply") => "Reply",
        (Language::Portugues, "menu.forward") => "Encaminhar",
        (_, "menu.forward") => "Forward",
        (Language::Portugues, "menu.copy") => "Copiar texto",
        (_, "menu.copy") => "Copy text",
        (Language::Portugues, "menu.edit") => "Editar",
        (_, "menu.edit") => "Edit",
        (Language::Portugues, "menu.delete_everyone") => "Apagar para todos",
        (_, "menu.delete_everyone") => "Delete for everyone",
        (Language::Portugues, "menu.delete_me") => "Apagar para mim",
        (_, "menu.delete_me") => "Delete for me",
        (Language::Portugues, "menu.save_sticker") => "Salvar figurinha",
        (_, "menu.save_sticker") => "Save sticker",
        (Language::Portugues, "menu.open_file") => "Abrir arquivo",
        (_, "menu.open_file") => "Open file",
        (Language::Portugues, "menu.download") => "Baixar",
        (_, "menu.download") => "Download",
        (Language::Portugues, "menu.show_folder") => "Mostrar na pasta",
        (_, "menu.show_folder") => "Show in folder",
        (Language::Portugues, "msginfo.sent") => "Enviada {stamp}",
        (_, "msginfo.sent") => "Sent {stamp}",
        (Language::Portugues, "msginfo.delivered") => "Entregue {stamp}",
        (_, "msginfo.delivered") => "Delivered {stamp}",
        (Language::Portugues, "msginfo.delivered_bare") => "Entregue",
        (_, "msginfo.delivered_bare") => "Delivered",
        (Language::Portugues, "msginfo.read") => "Lida {stamp}",
        (_, "msginfo.read") => "Read {stamp}",
        (Language::Portugues, "msginfo.read_bare") => "Lida",
        (_, "msginfo.read_bare") => "Read",
        (Language::Portugues, "msginfo.played") => "Ouvida {stamp}",
        (_, "msginfo.played") => "Played {stamp}",
        (Language::Portugues, "msginfo.played_bare") => "Ouvida",
        (_, "msginfo.played_bare") => "Played",
        (Language::Portugues, "reply.to") => "Respondendo a {who}",
        (_, "reply.to") => "Replying to {who}",
        // --- picker ---------------------------------------------------------
        (Language::Portugues, "picker.emoji") => "Emoji",
        (_, "picker.emoji") => "Emoji",
        (Language::Portugues, "picker.gifs") => "GIF",
        (_, "picker.gifs") => "GIF",
        (Language::Portugues, "picker.stickers") => "Figurinhas",
        (_, "picker.stickers") => "Stickers",
        (Language::Portugues, "picker.searching") => "Pesquisando…",
        (_, "picker.searching") => "Searching…",
        (Language::Portugues, "picker.gif_key_rejected") => {
            "Esta chave da API do GIPHY foi rejeitada. Crie uma chave grátis em developers.giphy.com e cole aqui. Ela fica salva nas suas configurações."
        }
        (_, "picker.gif_key_rejected") => {
            "This GIPHY API key was rejected. Create a free key at developers.giphy.com and paste it here. It is saved in your settings."
        }
        (Language::Portugues, "picker.gif_key_missing") => {
            "A busca de GIFs precisa de uma chave da API do GIPHY. Crie uma chave grátis em developers.giphy.com e cole aqui. Ela fica salva nas suas configurações."
        }
        (_, "picker.gif_key_missing") => {
            "GIF search needs a GIPHY API key. Create a free key at developers.giphy.com and paste it here. It is saved in your settings."
        }
        (Language::Portugues, "picker.gif_key_hint") => "Chave da API do GIPHY",
        (_, "picker.gif_key_hint") => "GIPHY API key",
        (Language::Portugues, "picker.gif_search") => "Pesquisar GIFs via GIPHY",
        (_, "picker.gif_search") => "Search GIFs via GIPHY",
        (Language::Portugues, "picker.gif_empty") => {
            "Pesquise um GIF ou veja os resultados em alta."
        }
        (_, "picker.gif_empty") => "Search for a GIF or browse trending results.",
        (Language::Portugues, "picker.stickers_loading") => "Carregando suas figurinhas…",
        (_, "picker.stickers_loading") => "Loading your stickers…",
        (Language::Portugues, "picker.stickers_empty") => {
            "Figurinhas recentes aparecem aqui. Clique com o botão direito em uma para salvá-la. Para importar um pacote, cole um link signal.art ou abra um arquivo .wastickers."
        }
        (_, "picker.stickers_empty") => {
            "Recent stickers appear here. Right-click one to save it. To import a pack, paste a signal.art link or open a .wastickers file."
        }
        (Language::Portugues, "picker.saved") => "Salvas",
        (_, "picker.saved") => "Saved",
        (Language::Portugues, "picker.recent") => "Recentes",
        (_, "picker.recent") => "Recent",
        (Language::Portugues, "picker.signal_hint") => "Cole um link signal.art",
        (_, "picker.signal_hint") => "Paste a signal.art link",
        (Language::Portugues, "picker.find_packs") => "Buscar pacotes",
        (_, "picker.find_packs") => "Find packs",
        (Language::Portugues, "picker.browse_packs") => "Ver signalstickers.org",
        (_, "picker.browse_packs") => "Browse signalstickers.org",
        (Language::Portugues, "picker.open_file") => "Abrir arquivo",
        (_, "picker.open_file") => "Open file",
        (Language::Portugues, "picker.importing") => "Importando o pacote…",
        (_, "picker.importing") => "Importing the pack…",
        (Language::Portugues, "picker.save") => "Salvar figurinha",
        (_, "picker.save") => "Save sticker",
        (Language::Portugues, "picker.unsave") => "Remover das salvas",
        (_, "picker.unsave") => "Remove from saved",
        (Language::Portugues, "picker.remove_pack") => "Remover este pacote",
        (_, "picker.remove_pack") => "Remove this pack",
        (Language::Portugues, "composer.cancel_reply") => "Cancelar resposta (Esc)",
        (_, "composer.cancel_reply") => "Cancel reply (Esc)",
        // --- presence / chat info -----------------------------------------
        (Language::Portugues, "presence.online") => "online",
        (_, "presence.online") => "online",
        (Language::Portugues, "presence.last_seen") => "visto por último em {stamp}",
        (_, "presence.last_seen") => "last seen {stamp}",
        (Language::Portugues, "chat.members") => "{n} membros",
        (_, "chat.members") => "{n} members",
        (Language::Portugues, "chat.muted") => "Silenciada",
        (_, "chat.muted") => "Muted",
        (Language::Portugues, "chat.muted_until") => "Silenciada até {stamp}",
        (_, "chat.muted_until") => "Muted until {stamp}",
        // --- dialogs ----------------------------------------------------------
        (Language::Portugues, "dialog.forward") => "Encaminhar mensagem",
        (_, "dialog.forward") => "Forward message",
        (Language::Portugues, "dialog.search_chats") => "Pesquisar conversas",
        (_, "dialog.search_chats") => "Search chats",
        (Language::Portugues, "dialog.no_chats") => "Nenhuma conversa disponível",
        (_, "dialog.no_chats") => "No writable chats found",
        (Language::Portugues, "dialog.close") => "Fechar",
        (_, "dialog.close") => "Close",
        (Language::Portugues, "dialog.shortcuts") => "Atalhos de teclado",
        (_, "dialog.shortcuts") => "Keyboard shortcuts",
        (Language::Portugues, "dialog.about") => "Sobre",
        (_, "dialog.about") => "About",
        (Language::Portugues, "dialog.version") => "Versão {v}",
        (_, "dialog.version") => "Version {v}",
        (Language::Portugues, "dialog.about_body") => {
            "Um cliente nativo de WhatsApp escrito em Rust com egui. Conecta-se através do whatsapp-rust. As mensagens têm criptografia de ponta a ponta neste aparelho."
        }
        (_, "dialog.about_body") => {
            "A native WhatsApp client written in Rust with egui. It connects through whatsapp-rust. Messages are end-to-end encrypted on this device."
        }
        (Language::Portugues, "dialog.about_unofficial") => {
            "Este é um cliente não oficial. Usá-lo pode violar os termos de serviço do WhatsApp e levar à suspensão da conta."
        }
        (_, "dialog.about_unofficial") => {
            "This is an unofficial client. Using it may be against WhatsApp's terms of service and could get an account suspended."
        }
        (Language::Portugues, "dialog.source") => "Código-fonte",
        (_, "dialog.source") => "Source code",
        (Language::Portugues, "dialog.unlink_title") => "Desvincular este computador?",
        (_, "dialog.unlink_title") => "Unlink this computer?",
        (Language::Portugues, "dialog.unlink_body") => {
            "Isso remove o aparelho do WhatsApp e apaga as conversas guardadas aqui. Você pode vincular de novo com um novo código."
        }
        (_, "dialog.unlink_body") => {
            "This removes the device from WhatsApp and deletes the chats stored here. You can link again with a new code."
        }
        (Language::Portugues, "dialog.unlink") => "Desvincular",
        (_, "dialog.unlink") => "Unlink",
        (Language::Portugues, "dialog.cancel") => "Cancelar",
        (_, "dialog.cancel") => "Cancel",
        (Language::Portugues, "dialog.pair_title") => "Vincular com um número de telefone",
        (_, "dialog.pair_title") => "Link with a phone number",
        (Language::Portugues, "dialog.pair_body") => {
            "Informe o número de WhatsApp com o código do país. Não inclua o sinal de mais nem zero à esquerda. Você vai receber um código para digitar no telefone."
        }
        (_, "dialog.pair_body") => {
            "Enter the WhatsApp phone number with its country code. Do not include a plus sign or leading zero. You will get a code to enter on the phone."
        }
        (Language::Portugues, "dialog.get_code") => "Obter um código",
        (_, "dialog.get_code") => "Get a code",
        (Language::Portugues, "dialog.new_contact") => "Novo contato",
        (_, "dialog.new_contact") => "New contact",
        (Language::Portugues, "dialog.new_contact_body") => {
            "Informe um número com o código do país, sem sinal de mais nem zero à esquerda. Adicione um nome para salvar o contato, ou deixe em branco para abrir a conversa. O WhatsApp usa o primeiro nome como nome de exibição."
        }
        (_, "dialog.new_contact_body") => {
            "Enter a phone number with its country code, without a plus sign or leading zero. Add a name to save the contact, or leave it blank to open the chat. WhatsApp uses the first name as the display name."
        }
        (Language::Portugues, "dialog.first_name") => "Nome",
        (_, "dialog.first_name") => "First name",
        (Language::Portugues, "dialog.last_name") => "Sobrenome",
        (_, "dialog.last_name") => "Surname",
        (Language::Portugues, "dialog.checking") => "Verificando o número…",
        (_, "dialog.checking") => "Checking the number…",
        (Language::Portugues, "dialog.save_contact") => "Salvar contato",
        (_, "dialog.save_contact") => "Save contact",
        (Language::Portugues, "dialog.group") => "Grupo",
        (_, "dialog.group") => "Group",
        (Language::Portugues, "dialog.message") => "Conversar",
        (_, "dialog.message") => "Message",
        (Language::Portugues, "dialog.contact") => "Contato",
        (_, "dialog.contact") => "Contact",
        (Language::Portugues, "dialog.save_name") => "Salvar nome (Enter)",
        (_, "dialog.save_name") => "Save name (Enter)",
        (Language::Portugues, "dialog.members") => "Membros ({n})",
        (_, "dialog.members") => "Members ({n})",
        (Language::Portugues, "dialog.rename") => "Renomear",
        (_, "dialog.rename") => "Rename",
        (Language::Portugues, "dialog.add_contacts") => "Adicionar aos contatos",
        (_, "dialog.add_contacts") => "Add to contacts",
        (Language::Portugues, "dialog.mute") => "Silenciar",
        (_, "dialog.mute") => "Mute",
        (Language::Portugues, "dialog.pin") => "Fixar",
        (_, "dialog.pin") => "Pin",
        // --- conversation -----------------------------------------------------
        (Language::Portugues, "conv.empty_loading") => {
            "Suas conversas aparecem à esquerda conforme carregam."
        }
        (_, "conv.empty_loading") => "Your chats appear on the left as they load.",
        (Language::Portugues, "conv.empty_select") => "Selecione uma conversa à esquerda.",
        (_, "conv.empty_select") => "Select a chat on the left.",
        (Language::Portugues, "conv.hints") => "Ctrl+K para pesquisar · Ctrl+/ para atalhos",
        (_, "conv.hints") => "Ctrl+K to search · Ctrl+/ for shortcuts",
        (Language::Portugues, "conv.show_list") => "Mostrar a lista de conversas (Ctrl+B)",
        (_, "conv.show_list") => "Show the chat list (Ctrl+B)",
        (Language::Portugues, "conv.more") => "Mais",
        (_, "conv.more") => "More",
        (Language::Portugues, "conv.close_chat") => "Fechar conversa",
        (_, "conv.close_chat") => "Close chat",
        (Language::Portugues, "conv.typing_one") => "{one} está digitando…",
        (_, "conv.typing_one") => "{one} is typing…",
        (Language::Portugues, "conv.typing_many") => "{rest} e {last} estão digitando…",
        (_, "conv.typing_many") => "{rest} and {last} are typing…",
        (Language::Portugues, "conv.typing") => "digitando…",
        (_, "conv.typing") => "typing…",
        (Language::Portugues, "conv.send_files") => "Enviar arquivos (ou solte-os na janela)",
        (_, "conv.send_files") => "Send files (or drop them on the window)",
        (Language::Portugues, "conv.poll") => "Criar enquete",
        (_, "conv.poll") => "Create poll",
        (Language::Portugues, "conv.stickers") => "Emoji, GIFs e figurinhas",
        (_, "conv.stickers") => "Emoji, GIFs, and stickers",
        (Language::Portugues, "conv.type_message") => "Digite uma mensagem",
        (_, "conv.type_message") => "Type a message",
        (Language::Portugues, "conv.caption") => "Adicione uma legenda",
        (_, "conv.caption") => "Add a caption",
        (Language::Portugues, "conv.record") => "Gravar uma mensagem de voz",
        (_, "conv.record") => "Record a voice message",
        (Language::Portugues, "conv.send") => "Enviar",
        (_, "conv.send") => "Send",
        (Language::Portugues, "conv.hint_enter") => {
            "Enter envia · Shift+Enter para nova linha · *negrito* _itálico_ ~riscado~ · Ctrl+V cola uma foto"
        }
        (_, "conv.hint_enter") => {
            "Enter sends · Shift+Enter for a new line · *bold* _italic_ ~strike~ · Ctrl+V pastes a picture"
        }
        (Language::Portugues, "conv.hint_ctrl") => {
            "Ctrl+Enter envia · *negrito* _itálico_ ~riscado~ · Ctrl+V cola uma foto"
        }
        (_, "conv.hint_ctrl") => {
            "Ctrl+Enter sends · *bold* _italic_ ~strike~ · Ctrl+V pastes a picture"
        }
        (Language::Portugues, "conv.hide_hints") => {
            "Ocultar dicas de atalho (restaurar em Configurações)"
        }
        (_, "conv.hide_hints") => "Hide shortcut hints (restore in Settings)",
        (Language::Portugues, "conv.all_shortcuts") => "Todos os atalhos ({keys})",
        (_, "conv.all_shortcuts") => "All shortcuts ({keys})",
        (Language::Portugues, "conv.editing") => "Editando mensagem",
        (_, "conv.editing") => "Editing message",
        (Language::Portugues, "conv.stop_edit") => "Parar de editar (Esc)",
        (_, "conv.stop_edit") => "Stop editing (Esc)",
        (Language::Portugues, "conv.newest") => "Mensagem mais nova",
        (_, "conv.newest") => "Newest message",
        (Language::Portugues, "conv.loading_phone") => {
            "Carregando mensagens mais antigas do seu telefone…"
        }
        (_, "conv.loading_phone") => "Loading older messages from your phone…",
        (Language::Portugues, "conv.loading_phone_short") => {
            "Carregando mensagens do seu telefone…"
        }
        (_, "conv.loading_phone_short") => "Loading messages from your phone…",
        (Language::Portugues, "conv.empty_chat") => "Nenhuma mensagem aqui ainda",
        (_, "conv.empty_chat") => "No messages here yet",
        (Language::Portugues, "conv.forwarded") => "Encaminhada",
        (_, "conv.forwarded") => "Forwarded",
        (Language::Portugues, "conv.remove_reaction") => "Remover sua reação",
        (_, "conv.remove_reaction") => "Remove your reaction",
        (Language::Portugues, "conv.react_any") => "Reagir com qualquer emoji",
        (_, "conv.react_any") => "React with any emoji",
        (Language::Portugues, "conv.open_map") => "Abrir no mapa",
        (_, "conv.open_map") => "Open in a map",
        (Language::Portugues, "conv.unsupported") => "Não suportada: {what}",
        (_, "conv.unsupported") => "Unsupported: {what}",
        (Language::Portugues, "conv.picture_fail") => {
            "Não foi possível mostrar esta foto. Clique para abri-la."
        }
        (_, "conv.picture_fail") => "Could not display this picture. Click to open it.",
        (Language::Portugues, "conv.download_fail") => {
            "Falha no download. Clique para tentar de novo."
        }
        (_, "conv.download_fail") => "Download failed. Click to retry.",
        (Language::Portugues, "conv.video") => "Vídeo",
        (_, "conv.video") => "Video",
        (Language::Portugues, "conv.pause") => "Pausar",
        (_, "conv.pause") => "Pause",
        (Language::Portugues, "conv.play") => "Ouvir",
        (_, "conv.play") => "Play",
        (Language::Portugues, "conv.speed") => "Velocidade de reprodução",
        (_, "conv.speed") => "Playback speed",
        (Language::Portugues, "conv.preparing_speed") => "Preparando velocidade de reprodução",
        (_, "conv.preparing_speed") => "Preparing playback speed",
        (Language::Portugues, "conv.discard") => "Descartar",
        (_, "conv.discard") => "Discard",
        (Language::Portugues, "conv.only") => "Apenas",
        (_, "conv.only") => "Only",
        (Language::Portugues, "conv.admins") => "administradores",
        (_, "conv.admins") => "admins",
        (Language::Portugues, "conv.admins_can_send") => "podem enviar mensagens",
        (_, "conv.admins_can_send") => "can send messages",
        (Language::Portugues, "status.sending") => "enviando",
        (_, "status.sending") => "sending",
        (Language::Portugues, "status.sent") => "enviada",
        (_, "status.sent") => "sent",
        (Language::Portugues, "status.delivered") => "entregue",
        (_, "status.delivered") => "delivered",
        (Language::Portugues, "status.read") => "lida",
        (_, "status.read") => "read",
        (Language::Portugues, "status.played") => "ouvida",
        (_, "status.played") => "played",
        (Language::Portugues, "status.failed") => "falhou",
        (_, "status.failed") => "failed",
        (Language::Portugues, "conv.edited") => "editada",
        (_, "conv.edited") => "edited",
        // --- polls ------------------------------------------------------------
        (Language::Portugues, "poll.create") => "Criar enquete",
        (_, "poll.create") => "Create poll",
        (Language::Portugues, "poll.close") => "Fechar",
        (_, "poll.close") => "Close",
        (Language::Portugues, "poll.question") => "Pergunta",
        (_, "poll.question") => "Question",
        (Language::Portugues, "poll.ask_hint") => "Faça uma pergunta",
        (_, "poll.ask_hint") => "Ask a question",
        (Language::Portugues, "poll.answers") => "Respostas",
        (_, "poll.answers") => "Answers",
        (Language::Portugues, "poll.answer_hint") => "Resposta {n}",
        (_, "poll.answer_hint") => "Answer {n}",
        (Language::Portugues, "poll.remove_answer") => "Remover resposta",
        (_, "poll.remove_answer") => "Remove answer",
        (Language::Portugues, "poll.add_answer") => "Adicionar resposta",
        (_, "poll.add_answer") => "Add answer",
        (Language::Portugues, "poll.multiple") => "Permitir várias respostas",
        (_, "poll.multiple") => "Allow multiple answers",
        (Language::Portugues, "poll.cancel") => "Cancelar",
        (_, "poll.cancel") => "Cancel",
        (Language::Portugues, "poll.sending") => "Enviando…",
        (_, "poll.sending") => "Sending…",
        (Language::Portugues, "poll.send") => "Enviar enquete",
        (_, "poll.send") => "Send poll",
        (Language::Portugues, "poll.select_one") => "Selecione uma resposta",
        (_, "poll.select_one") => "Select one answer",
        (Language::Portugues, "poll.select_many") => "Selecione as respostas",
        (_, "poll.select_many") => "Select answers",
        (Language::Portugues, "poll.sending_vote") => "Enviando voto…",
        (_, "poll.sending_vote") => "Sending vote…",
        (Language::Portugues, "poll.waiting_phone") => {
            "Aguardando seu telefone · votos anteriores podem estar faltando"
        }
        (_, "poll.waiting_phone") => "Waiting for your phone · earlier votes may be missing",
        (Language::Portugues, "poll.loading_votes") => {
            "Carregando votos anteriores do seu telefone…"
        }
        (_, "poll.loading_votes") => "Loading earlier votes from your phone…",
        (Language::Portugues, "poll.votes_missing") => {
            "Votos anteriores ainda não foram carregados"
        }
        (_, "poll.votes_missing") => "Earlier votes have not been loaded yet",
        (Language::Portugues, "poll.no_key") => "Chave de votação indisponível · use seu telefone",
        (_, "poll.no_key") => "Voting key unavailable · use your phone",
        (Language::Portugues, "poll.reconnect") => "Reconecte para votar",
        (_, "poll.reconnect") => "Reconnect to vote",
        (Language::Portugues, "poll.voter_one") => "voto",
        (_, "poll.voter_one") => "voter",
        (Language::Portugues, "poll.voter_many") => "votos",
        (_, "poll.voter_many") => "voters",
        (Language::Portugues, "poll.err_question") => "Escreva uma pergunta de até 255 caracteres.",
        (_, "poll.err_question") => "Enter a question of up to 255 characters.",
        (Language::Portugues, "poll.err_answers") => {
            "Adicione de 2 a 12 respostas, cada uma com 1 a 100 caracteres."
        }
        (_, "poll.err_answers") => "Add 2–12 answers, each with 1–100 characters.",
        (Language::Portugues, "poll.err_unique") => "Cada resposta precisa ser diferente.",
        (_, "poll.err_unique") => "Each answer must be different.",
        // --- login --------------------------------------------------------------
        (Language::Portugues, "login.tagline") => "Um cliente nativo de WhatsApp.",
        (_, "login.tagline") => "A native WhatsApp client.",
        (Language::Portugues, "login.connecting") => "Conectando ao WhatsApp…",
        (_, "login.connecting") => "Connecting to WhatsApp…",
        (Language::Portugues, "login.linked") => "Vinculado. Aguardando suas conversas…",
        (_, "login.linked") => "Linked. Waiting for your chats…",
        (Language::Portugues, "login.unlinked") => {
            "Este computador foi desvinculado do seu telefone. Pedindo um novo código."
        }
        (_, "login.unlinked") => {
            "This computer was unlinked from your phone. Requesting a new code."
        }
        (Language::Portugues, "login.requesting") => "Pedindo um novo código…",
        (_, "login.requesting") => "Requesting a new code…",
        (Language::Portugues, "login.retry") => "Tentar de novo",
        (_, "login.retry") => "Try again",
        (Language::Portugues, "login.requesting_for") => "Pedindo um código para +{phone}…",
        (_, "login.requesting_for") => "Requesting a code for +{phone}…",
        (Language::Portugues, "login.waiting_code") => "Aguardando um código do WhatsApp…",
        (_, "login.waiting_code") => "Waiting for a code from WhatsApp…",
        (Language::Portugues, "login.unofficial") => {
            "Cliente não oficial. Usá-lo pode violar os termos de serviço do WhatsApp."
        }
        (_, "login.unofficial") => {
            "Unofficial client. Using it may be against WhatsApp's terms of service."
        }
        (Language::Portugues, "login.link_title") => "Vincule este computador",
        (_, "login.link_title") => "Link this computer",
        (Language::Portugues, "login.step1") => "Abra o WhatsApp no seu telefone",
        (_, "login.step1") => "Open WhatsApp on your phone",
        (Language::Portugues, "login.step2") => {
            "Toque em Menu ou Configurações e em Aparelhos vinculados"
        }
        (_, "login.step2") => "Tap Menu or Settings, then Linked devices",
        (Language::Portugues, "login.step3_qr") => {
            "Toque em Vincular aparelho e aponte o telefone para este código"
        }
        (_, "login.step3_qr") => "Tap Link a device and point the phone at this code",
        (Language::Portugues, "login.step3_phone") => {
            "Toque em Vincular aparelho e depois em Vincular com número de telefone"
        }
        (_, "login.step3_phone") => "Tap Link a device, then Link with phone number instead",
        (Language::Portugues, "login.with_number") => "Vincular com número de telefone",
        (_, "login.with_number") => "Link with phone number instead",
        (Language::Portugues, "login.enter_code") => "Digite este código no seu telefone",
        (_, "login.enter_code") => "Enter this code on your phone",
        (Language::Portugues, "login.copy_code") => "Copiar código",
        (_, "login.copy_code") => "Copy code",
        // --- updates / banner -----------------------------------------------------
        (Language::Portugues, "update.title") => "Atualizar o ZapFast",
        (_, "update.title") => "Update ZapFast",
        (Language::Portugues, "update.close") => "Fechar atualização",
        (_, "update.close") => "Close update",
        (Language::Portugues, "update.notes") => "Notas de lançamento",
        (_, "update.notes") => "Release notes",
        (Language::Portugues, "update.checking") => "Verificando download…",
        (_, "update.checking") => "Checking download…",
        (Language::Portugues, "update.downloading") => "Baixando atualização…",
        (_, "update.downloading") => "Downloading update…",
        (Language::Portugues, "update.ready") => "Pronto para instalar",
        (_, "update.ready") => "Ready to install",
        (Language::Portugues, "update.restart_note") => {
            "Termine mensagens não enviadas ou gravações antes de reiniciar. O ZapFast vai desconectar rapidinho e reconectar sozinho."
        }
        (_, "update.restart_note") => {
            "Finish any unsent messages or recordings before restarting. ZapFast will briefly disconnect, then reconnect automatically."
        }
        (Language::Portugues, "update.restart") => "Reiniciar para atualizar",
        (_, "update.restart") => "Restart to update",
        (Language::Portugues, "update.preparing") => "Preparando para reiniciar…",
        (_, "update.preparing") => "Preparing to restart…",
        (Language::Portugues, "update.download_github") => "Baixar do GitHub",
        (_, "update.download_github") => "Download from GitHub",
        (Language::Portugues, "update.retry") => "Tentar baixar de novo",
        (_, "update.retry") => "Retry download",
        (Language::Portugues, "update.download") => "Baixar atualização",
        (_, "update.download") => "Download update",
        (Language::Portugues, "banner.drop") => "Solte para enviar para {name}",
        (_, "banner.drop") => "Drop to send to {name}",
        (Language::Portugues, "banner.loading") => "Carregando histórico de conversas…",
        (_, "banner.loading") => "Loading chat history…",
        (Language::Portugues, "banner.loading_pct") => "Carregando histórico de conversas… {pct}%",
        (_, "banner.loading_pct") => "Loading chat history… {pct}%",
        (Language::Portugues, "banner.available") => "ZapFast {v} disponível",
        (_, "banner.available") => "ZapFast {v} is available",
        (Language::Portugues, "banner.connecting") => "Conectando ao WhatsApp…",
        (_, "banner.connecting") => "Connecting to WhatsApp…",
        (Language::Portugues, "banner.offline") => "Offline ({reason}). Reconectando…",
        (_, "banner.offline") => "Offline ({reason}). Reconnecting…",
        (Language::Portugues, "banner.not_linked") => "Não vinculado a um telefone",
        (_, "banner.not_linked") => "Not linked to a phone",
        (Language::Portugues, "banner.update") => "Atualizar",
        (_, "banner.update") => "Update",
        (Language::Portugues, "banner.retry") => "Tentar de novo",
        (_, "banner.retry") => "Retry",
        // --- shortcuts --------------------------------------------------------
        (Language::Portugues, "shortcut.search") => "Pesquisar conversas",
        (_, "shortcut.search") => "Search chats",
        (Language::Portugues, "shortcut.focus_input") => "Focar o campo de mensagem",
        (_, "shortcut.focus_input") => "Focus the message input",
        (Language::Portugues, "shortcut.prev_next") => "Conversa anterior / próxima",
        (_, "shortcut.prev_next") => "Previous / next chat",
        (Language::Portugues, "shortcut.send") => "Enviar (Shift+Enter para nova linha)",
        (_, "shortcut.send") => "Send (Shift+Enter for a new line)",
        (Language::Portugues, "shortcut.escape") => {
            "Descartar a ação atual, sair da pesquisa ou fechar a conversa"
        }
        (_, "shortcut.escape") => {
            "Dismiss the current action, return from search, or close the chat"
        }
        (Language::Portugues, "shortcut.paste") => {
            "Colar texto, ou enviar uma foto da área de transferência"
        }
        (_, "shortcut.paste") => "Paste text, or send a picture from the clipboard",
        (Language::Portugues, "shortcut.sidebar") => "Mostrar ou ocultar a lista de conversas",
        (_, "shortcut.sidebar") => "Show or hide the chat list",
        (Language::Portugues, "shortcut.bottom") => "Pular para a mensagem mais nova",
        (_, "shortcut.bottom") => "Jump to the newest message",
        (Language::Portugues, "shortcut.settings") => "Configurações",
        (_, "shortcut.settings") => "Settings",
        (Language::Portugues, "shortcut.zoom") => "Aproximar / afastar",
        (_, "shortcut.zoom") => "Zoom in / out",
        (Language::Portugues, "shortcut.zoom_reset") => "Restaurar zoom",
        (_, "shortcut.zoom_reset") => "Reset zoom",
        (Language::Portugues, "shortcut.list") => "Esta lista",
        (_, "shortcut.list") => "This list",
        (Language::Portugues, "shortcut.close_window") => {
            "Fechar a janela (o ZapFast continua na bandeja)"
        }
        (_, "shortcut.close_window") => "Close the window (ZapFast remains in the tray)",
        (Language::Portugues, "shortcut.quit") => "Sair",
        (_, "shortcut.quit") => "Quit",
        // --- message summaries ------------------------------------------
        (Language::Portugues, "summary.photo") => "Foto",
        (_, "summary.photo") => "Photo",
        (Language::Portugues, "summary.gif") => "GIF",
        (_, "summary.gif") => "GIF",
        (Language::Portugues, "summary.video") => "Vídeo",
        (_, "summary.video") => "Video",
        (Language::Portugues, "summary.voice") => "Mensagem de voz",
        (_, "summary.voice") => "Voice message",
        (Language::Portugues, "summary.audio") => "Áudio",
        (_, "summary.audio") => "Audio",
        (Language::Portugues, "summary.document") => "Documento",
        (_, "summary.document") => "Document",
        (Language::Portugues, "summary.sticker") => "Figurinha",
        (_, "summary.sticker") => "Sticker",
        (Language::Portugues, "summary.location") => "Localização",
        (_, "summary.location") => "Location",
        (Language::Portugues, "summary.contact") => "Contato",
        (_, "summary.contact") => "Contact",
        (Language::Portugues, "summary.poll") => "Enquete",
        (_, "summary.poll") => "Poll",
        (Language::Portugues, "summary.deleted") => "Esta mensagem foi apagada",
        (_, "summary.deleted") => "This message was deleted",
        (Language::Portugues, "summary.unsupported") => "Mensagem não suportada",
        (_, "summary.unsupported") => "Unsupported message",
        // --- contact card -------------------------------------------------
        (Language::Portugues, "contact.add") => "Adicionar",
        (_, "contact.add") => "Add",
        (Language::Portugues, "contact.message") => "Enviar mensagem",
        (_, "contact.message") => "Message",
        (Language::Portugues, "contact.single") => "Contato",
        (_, "contact.single") => "Contact",
        (Language::Portugues, "contact.many") => "{n} contatos",
        (_, "contact.many") => "{n} contacts",
        // --- dates --------------------------------------------------------
        (Language::Portugues, "date.yesterday") => "Ontem",
        (_, "date.yesterday") => "Yesterday",
        (Language::Portugues, "date.today") => "Hoje",
        (_, "date.today") => "Today",
        (Language::Portugues, "date.weekday.monday") => "segunda-feira",
        (_, "date.weekday.monday") => "Monday",
        (Language::Portugues, "date.weekday.tuesday") => "terça-feira",
        (_, "date.weekday.tuesday") => "Tuesday",
        (Language::Portugues, "date.weekday.wednesday") => "quarta-feira",
        (_, "date.weekday.wednesday") => "Wednesday",
        (Language::Portugues, "date.weekday.thursday") => "quinta-feira",
        (_, "date.weekday.thursday") => "Thursday",
        (Language::Portugues, "date.weekday.friday") => "sexta-feira",
        (_, "date.weekday.friday") => "Friday",
        (Language::Portugues, "date.weekday.saturday") => "sábado",
        (_, "date.weekday.saturday") => "Saturday",
        (Language::Portugues, "date.weekday.sunday") => "domingo",
        (_, "date.weekday.sunday") => "Sunday",
        (Language::Portugues, "date.month.1") => "janeiro",
        (_, "date.month.1") => "January",
        (Language::Portugues, "date.month.2") => "fevereiro",
        (_, "date.month.2") => "February",
        (Language::Portugues, "date.month.3") => "março",
        (_, "date.month.3") => "March",
        (Language::Portugues, "date.month.4") => "abril",
        (_, "date.month.4") => "April",
        (Language::Portugues, "date.month.5") => "maio",
        (_, "date.month.5") => "May",
        (Language::Portugues, "date.month.6") => "junho",
        (_, "date.month.6") => "June",
        (Language::Portugues, "date.month.7") => "julho",
        (_, "date.month.7") => "July",
        (Language::Portugues, "date.month.8") => "agosto",
        (_, "date.month.8") => "August",
        (Language::Portugues, "date.month.9") => "setembro",
        (_, "date.month.9") => "September",
        (Language::Portugues, "date.month.10") => "outubro",
        (_, "date.month.10") => "October",
        (Language::Portugues, "date.month.11") => "novembro",
        (_, "date.month.11") => "November",
        (Language::Portugues, "date.month.12") => "dezembro",
        (_, "date.month.12") => "December",
        // --- toasts -------------------------------------------------------
        (Language::Portugues, "toast.history_loaded") => "Histórico carregado",
        (_, "toast.history_loaded") => "History loaded",
        (Language::Portugues, "toast.not_connected") => "Não conectado ao WhatsApp",
        (_, "toast.not_connected") => "Not connected to WhatsApp",
        (Language::Portugues, "toast.not_connected_yet") => "Ainda não conectado ao WhatsApp",
        (_, "toast.not_connected_yet") => "Not connected to WhatsApp yet",
        (Language::Portugues, "toast.link_phone_fail") => {
            "Não foi possível vincular pelo número de telefone: {error}"
        }
        (_, "toast.link_phone_fail") => "Could not link by phone number: {error}",
        (Language::Portugues, "toast.connect_fail") => {
            "A conexão com o WhatsApp falhou ({reason}{detail})"
        }
        (_, "toast.connect_fail") => "WhatsApp connection failed ({reason}{detail})",
        (Language::Portugues, "toast.stream_replaced") => {
            "Outra sessão do WhatsApp Web substituiu esta"
        }
        (_, "toast.stream_replaced") => "Another WhatsApp Web session replaced this one",
        (Language::Portugues, "toast.history_fail") => {
            "Não foi possível ler parte do histórico de conversas: {error}"
        }
        (_, "toast.history_fail") => "Could not read part of the chat history: {error}",
        (Language::Portugues, "toast.phone_silent") => {
            "Seu telefone não enviou mensagens mais antigas. Verifique se ele está online"
        }
        (_, "toast.phone_silent") => {
            "Your phone did not send older messages. Check that it is online"
        }
        (Language::Portugues, "toast.save_sticker_fail") => {
            "Não foi possível salvar a figurinha: {error}"
        }
        (_, "toast.save_sticker_fail") => "Could not save sticker: {error}",
        (Language::Portugues, "toast.pack_added") => "Pacote de figurinhas adicionado: {name}",
        (_, "toast.pack_added") => "Added sticker pack \"{name}\"",
        (Language::Portugues, "toast.pack_fail") => {
            "Não foi possível adicionar o pacote de figurinhas: {error}"
        }
        (_, "toast.pack_fail") => "Could not add sticker pack: {error}",
        (Language::Portugues, "toast.save_contact_fail") => {
            "Não foi possível salvar o contato: {error}"
        }
        (_, "toast.save_contact_fail") => "Could not save contact: {error}",
        (Language::Portugues, "toast.contact_added") => "{name} adicionado aos contatos",
        (_, "toast.contact_added") => "Added {name} to contacts",
        (Language::Portugues, "toast.send_fail") => "Mensagem não enviada: {error}",
        (_, "toast.send_fail") => "Message not sent: {error}",
        (Language::Portugues, "toast.cannot_forward") => "Esta mensagem não pode ser encaminhada",
        (_, "toast.cannot_forward") => "This message cannot be forwarded",
        (Language::Portugues, "toast.forward_data_missing") => {
            "Os dados originais da mensagem não estão disponíveis para encaminhar"
        }
        (_, "toast.forward_data_missing") => {
            "The original message data is not available to forward"
        }
        (Language::Portugues, "toast.forward_data_broken") => {
            "Os dados originais da mensagem não puderam ser lidos"
        }
        (_, "toast.forward_data_broken") => "The original message data could not be read",
        (Language::Portugues, "toast.not_stored") => {
            "Esta mensagem não está salva neste computador"
        }
        (_, "toast.not_stored") => "This message is not stored on this computer",
        (Language::Portugues, "toast.encode_attachment") => "Não foi possível codificar o anexo",
        (_, "toast.encode_attachment") => "Could not encode the attachment",
        (Language::Portugues, "toast.read_chat") => "Não foi possível ler a conversa: {error}",
        (_, "toast.read_chat") => "Could not read the chat: {error}",
        (Language::Portugues, "toast.search_fail") => "Não foi possível pesquisar: {error}",
        (_, "toast.search_fail") => "Could not search: {error}",
        // --- widgets --------------------------------------------------------------
        (Language::Portugues, "widgets.clear") => "Limpar",
        (_, "widgets.clear") => "Clear",
        (Language::Portugues, "widgets.unread") => "{label}, {count} não lidas",
        (_, "widgets.unread") => "{label}, {count} unread",
        // --- emoji --------------------------------------------------------------
        (Language::Portugues, "emoji.frequent") => "Usados com frequência",
        (_, "emoji.frequent") => "Frequently Used",
        (Language::Portugues, "emoji.no_match") => "Nada corresponde",
        (_, "emoji.no_match") => "Nothing matches",
        (Language::Portugues, "emoji.search") => "Pesquisar emoji",
        (_, "emoji.search") => "Search emoji",
        (Language::Portugues, "emoji.group.smileys") => "Emoticons e emoções",
        (_, "emoji.group.smileys") => "Smileys & Emotion",
        (Language::Portugues, "emoji.group.people") => "Pessoas e corpo",
        (_, "emoji.group.people") => "People & Body",
        (Language::Portugues, "emoji.group.animals") => "Animais e natureza",
        (_, "emoji.group.animals") => "Animals & Nature",
        (Language::Portugues, "emoji.group.food") => "Comida e bebida",
        (_, "emoji.group.food") => "Food & Drink",
        (Language::Portugues, "emoji.group.travel") => "Viagens e lugares",
        (_, "emoji.group.travel") => "Travel & Places",
        (Language::Portugues, "emoji.group.activities") => "Atividades",
        (_, "emoji.group.activities") => "Activities",
        (Language::Portugues, "emoji.group.objects") => "Objetos",
        (_, "emoji.group.objects") => "Objects",
        (Language::Portugues, "emoji.group.symbols") => "Símbolos",
        (_, "emoji.group.symbols") => "Symbols",
        (Language::Portugues, "emoji.group.flags") => "Bandeiras",
        (_, "emoji.group.flags") => "Flags",
        (Language::Portugues, "toast.back_online") => "De volta online",
        (_, "toast.back_online") => "Back online",
        (Language::Portugues, "toast.unlinked") => "Este aparelho foi desvinculado do seu telefone",
        (_, "toast.unlinked") => "This device was unlinked from your phone",
        (Language::Portugues, "toast.open_chat_first") => "Abra uma conversa primeiro",
        (_, "toast.open_chat_first") => "Open a chat first",
        (Language::Portugues, "toast.open_fail") => "Não foi possível abrir {path}: {error}",
        (_, "toast.open_fail") => "Could not open {path}: {error}",
        (Language::Portugues, "toast.copied") => "Copiado",
        (_, "toast.copied") => "Copied",
        (Language::Portugues, "toast.sticker_saved") => "Figurinha salva",
        (_, "toast.sticker_saved") => "Sticker saved",
        (Language::Portugues, "toast.sending_gif") => "Enviando GIF…",
        (_, "toast.sending_gif") => "Sending GIF…",
        (Language::Portugues, "toast.pair_hint") => {
            "Informe o número de telefone com o código do país, só com dígitos"
        }
        (_, "toast.pair_hint") => "Enter the phone number with its country code, using digits only",
        (Language::Portugues, "toast.update_available") => "ZapFast {v} disponível",
        (_, "toast.update_available") => "ZapFast {v} is available",
        (Language::Portugues, "bubble.expired") => "Não disponível mais nos servidores do WhatsApp",
        (_, "bubble.expired") => "No longer available on WhatsApp's servers",
        (Language::Portugues, "toast.sending_one") => "Enviando 1 arquivo…",
        (_, "toast.sending_one") => "Sending 1 file…",
        (Language::Portugues, "toast.sending_many") => "Enviando {n} arquivos…",
        (_, "toast.sending_many") => "Sending {n} files…",
        (Language::Portugues, "toast.record_fail") => "Não foi possível gravar: {error}",
        (_, "toast.record_fail") => "Could not record: {error}",
        // --- polls ----------------------------------------------------------
        (Language::Portugues, "poll.no_broadcast") => {
            "Enquetes não podem ser enviadas para esta conversa."
        }
        (_, "poll.no_broadcast") => "Polls cannot be sent to this chat.",
        (Language::Portugues, "poll.no_ephemeral") => {
            "Criação de enquete em conversas com mensagens temporárias ainda não é suportada."
        }
        (_, "poll.no_ephemeral") => {
            "Poll creation in disappearing-message chats is not supported yet."
        }
        (Language::Portugues, "poll.recipients_fail") => {
            "Não foi possível carregar os participantes do grupo"
        }
        (_, "poll.recipients_fail") => "Could not load the group recipients",
        (Language::Portugues, "poll.send_fail") => {
            "Não foi possível enviar a enquete. Tente novamente."
        }
        (_, "poll.send_fail") => "Could not send the poll. Please try again.",
        (Language::Portugues, "poll.account_changed") => {
            "A conta mudou enquanto a enquete era enviada."
        }
        (_, "poll.account_changed") => "The account changed while the poll was being sent.",
        (Language::Portugues, "poll.key_fail") => {
            "A enquete foi enviada, mas sua chave de votação não pôde ser salva."
        }
        (_, "poll.key_fail") => "The poll was sent, but its voting key could not be saved.",
        (Language::Portugues, "poll.not_ready") => {
            "Esta enquete não está pronta para votação. Reconecte e tente de novo."
        }
        (_, "poll.not_ready") => "This poll is not ready for voting. Reconnect and try again.",
        (Language::Portugues, "poll.vote_fail") => {
            "Não foi possível enviar seu voto. Tente novamente."
        }
        (_, "poll.vote_fail") => "Could not send your vote. Please try again.",
        (Language::Portugues, "poll.vote_unsaved") => {
            "Seu voto foi enviado, mas não pôde ser salvo localmente."
        }
        (_, "poll.vote_unsaved") => "Your vote was sent, but could not be saved locally.",
        // --- link screen ----------------------------------------------------
        (Language::Portugues, "link.banned") => {
            "O WhatsApp bloqueou temporariamente esta conta ({code})"
        }
        (_, "link.banned") => "WhatsApp has temporarily blocked this account ({code})",
        (Language::Portugues, "link.outdated") => {
            "O WhatsApp rejeitou esta versão do ZapFast. Atualize o aplicativo"
        }
        (_, "link.outdated") => "WhatsApp rejected this version of ZapFast. Update the app",
        // --- fallback -----------------------------------------------------
        (_, _) => return None,
    })
}

/// Replaces `{name}`-style tokens in a template from [`t`].
pub fn fill(template: &str, replacements: &[(&str, &str)]) -> String {
    let mut out = template.to_owned();
    for (key, value) in replacements {
        out = out.replace(&format!("{{{key}}}"), value);
    }
    out
}

/// Translated weekday name for date stamps.
pub fn weekday(lang: Language, weekday: jiff::civil::Weekday) -> &'static str {
    match weekday {
        jiff::civil::Weekday::Monday => t(lang, "date.weekday.monday"),
        jiff::civil::Weekday::Tuesday => t(lang, "date.weekday.tuesday"),
        jiff::civil::Weekday::Wednesday => t(lang, "date.weekday.wednesday"),
        jiff::civil::Weekday::Thursday => t(lang, "date.weekday.thursday"),
        jiff::civil::Weekday::Friday => t(lang, "date.weekday.friday"),
        jiff::civil::Weekday::Saturday => t(lang, "date.weekday.saturday"),
        jiff::civil::Weekday::Sunday => t(lang, "date.weekday.sunday"),
    }
}

/// Translated month name for date stamps.
pub fn month(lang: Language, month: i8) -> &'static str {
    match month {
        1 => t(lang, "date.month.1"),
        2 => t(lang, "date.month.2"),
        3 => t(lang, "date.month.3"),
        4 => t(lang, "date.month.4"),
        5 => t(lang, "date.month.5"),
        6 => t(lang, "date.month.6"),
        7 => t(lang, "date.month.7"),
        8 => t(lang, "date.month.8"),
        9 => t(lang, "date.month.9"),
        10 => t(lang, "date.month.10"),
        11 => t(lang, "date.month.11"),
        _ => t(lang, "date.month.12"),
    }
}

/// Renders a stored English preview (`LastMessage.summary`, `Quoted.summary`)
/// in `lang` using its label key. Plain text (`None`) shows verbatim; user
/// content after the label is never translated. Falls back to the stored
/// string when the shape is unexpected (legacy rows).
pub fn preview(lang: Language, label: Option<&str>, summary: &str) -> String {
    let Some(key) = label else {
        return summary.to_owned();
    };
    if lang == Language::English {
        return summary.to_owned();
    }
    let en = t(Language::English, key);
    if en == key {
        return summary.to_owned();
    }
    let local = t(lang, key);
    if summary == en {
        return local.to_owned();
    }
    // Shapes from `Content::summary`: `Label: rest` and `Label (rest)`.
    if let Some(rest) = summary.strip_prefix(&format!("{en}: ")) {
        return format!("{local}: {rest}");
    }
    if summary.ends_with(')')
        && let Some(rest) = summary.strip_prefix(&format!("{en} ("))
    {
        return format!("{local} ({rest}");
    }
    summary.to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every key in the table, used to check both languages.
    const ALL_KEYS: &[&str] = &[
        "language.name",
        "person.you",
        "person.unsaved",
        "settings.title",
        "settings.back",
        "settings.section.appearance",
        "settings.theme",
        "settings.theme.follow_system",
        "settings.theme.dark",
        "settings.theme.light",
        "settings.theme.omarchy",
        "settings.theme.system",
        "settings.open_themes",
        "settings.zoom",
        "settings.zoom.hint",
        "settings.zoom.in",
        "settings.zoom.out",
        "settings.language",
        "settings.language.hint",
        "settings.section.chats",
        "settings.enter_sends",
        "settings.enter_sends.hint",
        "settings.read_receipts",
        "settings.read_receipts.off",
        "settings.read_receipts.on",
        "settings.typing",
        "settings.auto_download",
        "settings.auto_download.hint",
        "settings.sender_pictures",
        "settings.sender_pictures.hint",
        "settings.names_from_contacts",
        "settings.names_from_contacts.hint",
        "settings.save_to_phone",
        "settings.save_to_phone.hint",
        "settings.shortcut_hints",
        "settings.section.window",
        "settings.keep_running",
        "settings.keep_running.hint",
        "settings.notifications",
        "settings.notifications.hint",
        "settings.auto_update",
        "settings.auto_update.hint",
        "settings.check_updates",
        "settings.check_updates.hint",
        "settings.giphy",
        "settings.giphy.builtin",
        "settings.giphy.missing",
        "settings.section.account",
        "settings.linked_device",
        "settings.unlink",
        "settings.section.files",
        "settings.archive",
        "settings.open_folder",
        "settings.media",
        "settings.log",
        "settings.open",
        "settings.section.about",
        "settings.about.hint",
        "settings.about",
        "settings.shortcuts",
        "chats.title",
        "chats.archived",
        "chats.back",
        "chats.you",
        "chats.settings",
        "chats.new_contact",
        "chats.new_contact_mac",
        "chats.hide",
        "chats.hide_mac",
        "chats.search",
        "filter.all",
        "filter.unread",
        "filter.private",
        "filter.groups",
        "chats.empty.archived.title",
        "chats.empty.archived.body",
        "chats.empty.unread",
        "chats.empty.private",
        "chats.empty.groups",
        "chats.empty.filtered",
        "chats.empty.loading.title",
        "chats.empty.loading.body",
        "chats.empty.title",
        "chats.empty.body",
        "chats.no_results.title",
        "chats.no_results.body",
        "search.chats",
        "search.messages",
        "search.contacts",
        "menu.mark_read",
        "menu.unpin",
        "menu.pin",
        "menu.unarchive",
        "menu.archive",
        "menu.unmute",
        "menu.mute.8h",
        "menu.mute.week",
        "menu.mute.forever",
        "menu.copy_number",
        "menu.info",
        "menu.reply",
        "menu.forward",
        "menu.copy",
        "menu.edit",
        "menu.delete_everyone",
        "menu.delete_me",
        "menu.save_sticker",
        "menu.open_file",
        "menu.download",
        "menu.show_folder",
        "msginfo.sent",
        "msginfo.delivered",
        "msginfo.delivered_bare",
        "msginfo.read",
        "msginfo.read_bare",
        "msginfo.played",
        "msginfo.played_bare",
        "reply.to",
        "composer.cancel_reply",
        "picker.emoji",
        "picker.gifs",
        "picker.stickers",
        "picker.searching",
        "picker.gif_key_rejected",
        "picker.gif_key_missing",
        "picker.gif_key_hint",
        "picker.gif_search",
        "picker.gif_empty",
        "picker.stickers_loading",
        "picker.stickers_empty",
        "picker.saved",
        "picker.recent",
        "picker.signal_hint",
        "picker.find_packs",
        "picker.browse_packs",
        "picker.open_file",
        "picker.importing",
        "picker.save",
        "picker.unsave",
        "picker.remove_pack",
        "presence.online",
        "presence.last_seen",
        "chat.members",
        "chat.muted",
        "chat.muted_until",
        "dialog.message",
        "dialog.forward",
        "dialog.search_chats",
        "dialog.no_chats",
        "dialog.close",
        "dialog.shortcuts",
        "dialog.about",
        "dialog.version",
        "dialog.about_body",
        "dialog.about_unofficial",
        "dialog.source",
        "dialog.unlink_title",
        "dialog.unlink_body",
        "dialog.unlink",
        "dialog.cancel",
        "dialog.pair_title",
        "dialog.pair_body",
        "dialog.get_code",
        "dialog.new_contact",
        "dialog.new_contact_body",
        "dialog.first_name",
        "dialog.last_name",
        "dialog.checking",
        "dialog.save_contact",
        "dialog.group",
        "dialog.contact",
        "dialog.save_name",
        "dialog.members",
        "dialog.rename",
        "dialog.add_contacts",
        "dialog.mute",
        "dialog.pin",
        "conv.empty_loading",
        "conv.empty_select",
        "conv.hints",
        "conv.show_list",
        "conv.more",
        "conv.close_chat",
        "conv.typing_one",
        "conv.typing_many",
        "conv.typing",
        "conv.send_files",
        "conv.poll",
        "conv.stickers",
        "conv.type_message",
        "conv.caption",
        "conv.record",
        "conv.send",
        "conv.hint_enter",
        "conv.hint_ctrl",
        "conv.hide_hints",
        "conv.all_shortcuts",
        "conv.editing",
        "conv.stop_edit",
        "conv.newest",
        "conv.loading_phone",
        "conv.loading_phone_short",
        "conv.empty_chat",
        "conv.forwarded",
        "conv.remove_reaction",
        "conv.react_any",
        "conv.open_map",
        "conv.unsupported",
        "conv.picture_fail",
        "conv.download_fail",
        "conv.video",
        "conv.pause",
        "conv.play",
        "conv.speed",
        "conv.preparing_speed",
        "conv.discard",
        "conv.only",
        "conv.admins",
        "conv.admins_can_send",
        "status.sending",
        "status.sent",
        "status.delivered",
        "status.read",
        "status.played",
        "status.failed",
        "conv.edited",
        "poll.create",
        "poll.close",
        "poll.question",
        "poll.ask_hint",
        "poll.answers",
        "poll.answer_hint",
        "poll.remove_answer",
        "poll.add_answer",
        "poll.multiple",
        "poll.cancel",
        "poll.sending",
        "poll.send",
        "poll.select_one",
        "poll.select_many",
        "poll.sending_vote",
        "poll.waiting_phone",
        "poll.loading_votes",
        "poll.votes_missing",
        "poll.no_key",
        "poll.reconnect",
        "poll.voter_one",
        "poll.voter_many",
        "poll.err_question",
        "poll.err_answers",
        "poll.err_unique",
        "login.tagline",
        "login.connecting",
        "login.linked",
        "login.unlinked",
        "login.requesting",
        "login.retry",
        "login.requesting_for",
        "login.waiting_code",
        "login.unofficial",
        "login.link_title",
        "login.step1",
        "login.step2",
        "login.step3_qr",
        "login.step3_phone",
        "login.with_number",
        "login.enter_code",
        "login.copy_code",
        "update.title",
        "update.close",
        "update.notes",
        "update.checking",
        "update.downloading",
        "update.ready",
        "update.restart_note",
        "update.restart",
        "update.preparing",
        "update.download_github",
        "update.retry",
        "update.download",
        "banner.drop",
        "banner.loading",
        "banner.loading_pct",
        "banner.available",
        "banner.connecting",
        "banner.offline",
        "banner.not_linked",
        "banner.update",
        "banner.retry",
        "shortcut.search",
        "shortcut.focus_input",
        "shortcut.prev_next",
        "shortcut.send",
        "shortcut.escape",
        "shortcut.paste",
        "shortcut.sidebar",
        "shortcut.bottom",
        "shortcut.settings",
        "shortcut.zoom",
        "shortcut.zoom_reset",
        "shortcut.list",
        "shortcut.close_window",
        "shortcut.quit",
        "summary.photo",
        "summary.gif",
        "summary.video",
        "summary.voice",
        "summary.audio",
        "summary.document",
        "summary.sticker",
        "summary.location",
        "summary.contact",
        "summary.poll",
        "summary.deleted",
        "summary.unsupported",
        "contact.add",
        "contact.message",
        "contact.single",
        "contact.many",
        "date.yesterday",
        "date.today",
        "date.weekday.monday",
        "date.weekday.tuesday",
        "date.weekday.wednesday",
        "date.weekday.thursday",
        "date.weekday.friday",
        "date.weekday.saturday",
        "date.weekday.sunday",
        "date.month.1",
        "date.month.2",
        "date.month.3",
        "date.month.4",
        "date.month.5",
        "date.month.6",
        "date.month.7",
        "date.month.8",
        "date.month.9",
        "date.month.10",
        "date.month.11",
        "date.month.12",
        "toast.history_loaded",
        "toast.not_connected",
        "toast.not_connected_yet",
        "toast.link_phone_fail",
        "toast.connect_fail",
        "toast.stream_replaced",
        "toast.history_fail",
        "toast.phone_silent",
        "toast.save_sticker_fail",
        "toast.pack_added",
        "toast.pack_fail",
        "toast.save_contact_fail",
        "toast.contact_added",
        "toast.send_fail",
        "toast.cannot_forward",
        "toast.forward_data_missing",
        "toast.forward_data_broken",
        "toast.not_stored",
        "toast.encode_attachment",
        "toast.read_chat",
        "toast.search_fail",
        "widgets.clear",
        "widgets.unread",
        "emoji.frequent",
        "emoji.no_match",
        "emoji.search",
        "emoji.group.smileys",
        "emoji.group.people",
        "emoji.group.animals",
        "emoji.group.food",
        "emoji.group.travel",
        "emoji.group.activities",
        "emoji.group.objects",
        "emoji.group.symbols",
        "emoji.group.flags",
        "toast.back_online",
        "toast.unlinked",
        "toast.open_chat_first",
        "toast.open_fail",
        "toast.copied",
        "toast.sticker_saved",
        "toast.sending_gif",
        "toast.pair_hint",
        "toast.update_available",
        "bubble.expired",
        "toast.sending_one",
        "toast.sending_many",
        "toast.record_fail",
        "poll.no_broadcast",
        "poll.no_ephemeral",
        "poll.recipients_fail",
        "poll.send_fail",
        "poll.account_changed",
        "poll.key_fail",
        "poll.not_ready",
        "poll.vote_fail",
        "poll.vote_unsaved",
        "link.banned",
        "link.outdated",
    ];

    /// Every key must resolve in both languages without falling back to
    /// the key itself.
    #[test]
    fn every_key_exists_in_both_languages() {
        for key in ALL_KEYS {
            for lang in Language::ALL {
                assert_ne!(t(lang, key), *key, "missing {lang:?} string for {key}");
            }
        }
    }

    #[test]
    fn portuguese_differs_where_expected() {
        assert_eq!(t(Language::Portugues, "settings.title"), "Configurações");
        assert_eq!(t(Language::English, "settings.title"), "Settings");
    }
}
