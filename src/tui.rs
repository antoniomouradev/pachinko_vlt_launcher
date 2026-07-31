use anyhow::Result;
use crossterm::{
    event::KeyCode,
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    prelude::*,
    widgets::{Block, Borders, List, ListItem, Paragraph},
};
use std::cell::{Cell, RefCell};
use std::io;
use std::time::{Duration, Instant};

/// Não tenta reconectar mais que 1x/s — evita martelar o buttonhub se ele
/// cair de verdade, mas ainda recupera sozinho (ver comentário abaixo).
const BUTTONHUB_RETRY_INTERVAL: Duration = Duration::from_secs(1);

thread_local! {
    /// Conexão com o buttonhub, reusada por todo `poll_key`/`select` do
    /// processo (sempre chamados da thread principal, síncrona). `None` se
    /// não conseguiu conectar ainda (ex: rodando fora da VLT, em dev, ou
    /// buttonhub.service ainda subindo no boot) — nesse caso navegação cai
    /// só no teclado. Achado real: isso costumava ser um `OnceCell`, que
    /// cravava a falha da primeira tentativa pra sempre — se o launcher
    /// ganhava a corrida de boot contra o buttonhub.service, o menu nunca
    /// mais respondia a botão nenhum até reiniciar o processo. Agora
    /// reconecta sozinho.
    static BUTTONHUB: RefCell<Option<crate::hardware::buttonhub::Connection>> = RefCell::new(None);
    static BUTTONHUB_LAST_ATTEMPT: Cell<Option<Instant>> = Cell::new(None);
}

fn with_buttonhub_events<T>(f: impl FnOnce(Option<&std::sync::mpsc::Receiver<String>>) -> T) -> T {
    BUTTONHUB.with(|cell| {
        let mut conn = cell.borrow_mut();
        if conn.is_none() {
            let should_retry = BUTTONHUB_LAST_ATTEMPT.with(|last| {
                let now = Instant::now();
                let ready = last.get().map_or(true, |t| now.duration_since(t) >= BUTTONHUB_RETRY_INTERVAL);
                if ready {
                    last.set(Some(now));
                }
                ready
            });
            if should_retry {
                *conn = crate::hardware::buttonhub::connect(crate::hardware::buttonhub::DEFAULT_PORT).ok();
            }
        }
        f(conn.as_ref().map(|c| &c.events))
    })
}

/// Mapeamento das 4 teclas físicas usadas pra navegar o TUI (`ki`/`ke`/`ka`/`kg`
/// vindas do buttonhub). Maiúscula (`KI` etc) é release — ignorada, só reage
/// a press. Qualquer outra tecla física (noteiro `$`, heartbeat `!`, demais
/// `kX`) não navega nada, fica livre pra função do jogo.
fn map_button(line: &str) -> Option<KeyCode> {
    match line {
        "ki" => Some(KeyCode::Enter),
        "ke" => Some(KeyCode::Esc),
        "ka" => Some(KeyCode::Up),
        "kg" => Some(KeyCode::Down),
        _ => None,
    }
}

pub enum MenuChoice {
    TestMachine,
    UpdateLauncher,
    ConfigureMachine,
    Restart,
    Shutdown,
}

pub enum TestChoice {
    Inputs,
    Audio,
    Video,
    Connection,
}

const MENU_ITEMS: [&str; 5] =
    ["Testar Máquina", "Atualizar Launcher", "Registrar Máquina", "Reiniciar", "Desligar"];

const TEST_ITEMS: [&str; 4] = ["Testar Inputs", "Som", "Vídeo", "Conexão"];

fn with_screen<F, T>(f: F) -> Result<T>
where
    F: FnOnce(&mut Terminal<CrosstermBackend<io::Stdout>>) -> Result<T>,
{
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let mut terminal = Terminal::new(CrosstermBackend::new(stdout))?;
    // EnterAlternateScreen troca de buffer, mas não garante buffer em branco
    // em todo terminal — sem isso sobra resíduo da tela anterior (texto de
    // compilação, cor de um teste anterior etc.)
    terminal.clear()?;

    let result = f(&mut terminal);

    terminal.clear()?;
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    result
}

/// Menu de lista genérico: setas navegam, Enter escolhe (retorna o índice),
/// Esc volta `None`. Só as 4 teclas físicas do buttonhub (`ki`/`ke`/`ka`/`kg`,
/// ver `map_button`) navegam — sem teclado externo, lidas por `poll_key`.
pub fn select(title: &str, items: &[&str]) -> Result<Option<usize>> {
    with_screen(|terminal| {
        let mut selected = 0usize;

        let choice = loop {
            terminal.draw(|f| {
                let block = Block::default()
                    .title(format!(" {} ", title))
                    .borders(Borders::ALL);
                let list_items: Vec<ListItem> = items
                    .iter()
                    .enumerate()
                    .map(|(i, s)| {
                        let style = if i == selected {
                            Style::default().add_modifier(Modifier::REVERSED)
                        } else {
                            Style::default()
                        };
                        ListItem::new(*s).style(style)
                    })
                    .collect();
                f.render_widget(List::new(list_items).block(block), f.area());
            })?;

            match poll_key(Duration::from_millis(200))? {
                Some(KeyCode::Up) => selected = selected.saturating_sub(1),
                Some(KeyCode::Down) => selected = (selected + 1).min(items.len() - 1),
                Some(KeyCode::Enter) => break Some(selected),
                Some(KeyCode::Esc) => break None,
                _ => {}
            }
        };

        Ok(choice)
    })
}

/// Fonte grande em blocos (5 linhas) pra cada dígito 0-9 — o número digitado
/// precisa ser lido de longe na VLT física, texto normal de terminal é
/// pequeno demais.
const DIGIT_GLYPHS: [[&str; 5]; 10] = [
    [" ███ ", "█   █", "█   █", "█   █", " ███ "], // 0
    ["  █  ", " ██  ", "  █  ", "  █  ", " ███ "], // 1
    [" ███ ", "█   █", "   █ ", "  █  ", "█████"], // 2
    ["█████", "    █", "  ██ ", "    █", "█████"], // 3
    ["█   █", "█   █", "█████", "    █", "    █"], // 4
    ["█████", "█    ", "████ ", "    █", "████ "], // 5
    [" ███ ", "█    ", "████ ", "█   █", " ███ "], // 6
    ["█████", "    █", "   █ ", "  █  ", "  █  "], // 7
    [" ███ ", "█   █", " ███ ", "█   █", " ███ "], // 8
    [" ███ ", "█   █", " ████", "    █", " ███ "], // 9
];

/// Entrada numérica dígito a dígito, só com as 2 teclas físicas que existem
/// pra isso: `ka` (seta pra cima) soma 1 no dígito atual (0-9, dá a volta
/// pro 0 depois do 9), `ki` (Enter) confirma o dígito e avança pro próximo;
/// no último dígito, confirma a entrada inteira. `ke` (Esc) cancela tudo.
/// Sem tecla de "voltar" — errou dígito, digita até dar a volta de novo.
/// Dígitos em fonte grande, centralizados na tela (ver `DIGIT_GLYPHS`).
pub fn enter_digits(title: &str, num_digits: usize) -> Result<Option<String>> {
    with_screen(|terminal| {
        let mut digits = vec![0u8; num_digits];
        let mut pos = 0usize;

        let result = loop {
            terminal.draw(|f| {
                let area = f.area();
                let block = Block::default()
                    .title(format!(" {} ", title))
                    .borders(Borders::ALL);
                let inner = block.inner(area);
                f.render_widget(block, area);

                let chunks = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([
                        Constraint::Min(1),
                        Constraint::Length(5),
                        Constraint::Length(2),
                        Constraint::Min(1),
                    ])
                    .split(inner);

                let glyph_lines: Vec<Line> = (0..5)
                    .map(|row| {
                        let mut spans = Vec::with_capacity(digits.len() * 2);
                        for (i, &d) in digits.iter().enumerate() {
                            let style = if i == pos {
                                Style::default().add_modifier(Modifier::BOLD).fg(Color::Yellow)
                            } else {
                                Style::default()
                            };
                            spans.push(Span::styled(DIGIT_GLYPHS[d as usize][row], style));
                            spans.push(Span::raw("  "));
                        }
                        Line::from(spans)
                    })
                    .collect();

                f.render_widget(Paragraph::new(glyph_lines).alignment(Alignment::Center), chunks[1]);
                f.render_widget(
                    Paragraph::new("seta pra cima muda o dígito, Enter confirma e avança")
                        .alignment(Alignment::Center),
                    chunks[2],
                );
            })?;

            match poll_key(Duration::from_millis(200))? {
                Some(KeyCode::Up) => digits[pos] = (digits[pos] + 1) % 10,
                Some(KeyCode::Enter) => {
                    if pos + 1 == num_digits {
                        let value: String = digits.iter().map(|d| d.to_string()).collect();
                        break Some(value);
                    }
                    pos += 1;
                }
                Some(KeyCode::Esc) => break None,
                _ => {}
            }
        };

        Ok(result)
    })
}

pub fn run_menu() -> Result<Option<MenuChoice>> {
    let choice = select(" Pachinko3 ", &MENU_ITEMS)?;
    Ok(choice.map(|i| match i {
        0 => MenuChoice::TestMachine,
        1 => MenuChoice::UpdateLauncher,
        2 => MenuChoice::ConfigureMachine,
        3 => MenuChoice::Restart,
        _ => MenuChoice::Shutdown,
    }))
}

/// Submenu "Testar Máquina". Esc/q volta pro menu principal (`None`).
pub fn run_test_menu() -> Result<Option<TestChoice>> {
    let choice = select("Testar Máquina", &TEST_ITEMS)?;
    Ok(choice.map(|i| match i {
        0 => TestChoice::Inputs,
        1 => TestChoice::Audio,
        2 => TestChoice::Video,
        _ => TestChoice::Connection,
    }))
}

pub type Screen = Terminal<CrosstermBackend<io::Stdout>>;

/// Versões de baixo nível de `with_screen`/`select`, pra telas que precisam
/// intercalar `.await` (ex: ping em loop) entre um desenho e outro — não dá
/// pra usar o fechamento síncrono de `with_screen` nesse caso.
pub fn enter_screen() -> Result<Screen> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let mut screen = Terminal::new(CrosstermBackend::new(stdout))?;
    screen.clear()?;
    Ok(screen)
}

pub fn leave_screen(mut screen: Screen) -> Result<()> {
    screen.clear()?;
    disable_raw_mode()?;
    execute!(screen.backend_mut(), LeaveAlternateScreen)?;
    Ok(())
}

/// Tecla pressionada dentro de `timeout`, se houver (`None` = nada ainda).
/// Só o buttonhub (`ki`/`ke`/`ka`/`kg`, ver `map_button`) conta — sem teclado
/// externo. Sem buttonhub conectado, nunca retorna tecla nenhuma.
pub fn poll_key(timeout: Duration) -> Result<Option<KeyCode>> {
    let deadline = std::time::Instant::now() + timeout;

    loop {
        let from_buttonhub = with_buttonhub_events(|rx| {
            rx.and_then(|rx| rx.try_iter().find_map(|line| map_button(&line)))
        });
        if from_buttonhub.is_some() {
            return Ok(from_buttonhub);
        }

        if std::time::Instant::now() >= deadline {
            return Ok(None);
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

pub fn draw_lines(screen: &mut Screen, title: &str, lines: &[String]) -> Result<()> {
    screen.draw(|f| {
        let block = Block::default()
            .title(format!(" {} ", title))
            .borders(Borders::ALL);
        f.render_widget(Paragraph::new(lines.join("\n")).block(block), f.area());
    })?;
    Ok(())
}

const VIDEO_COLORS: [(&str, Color); 7] = [
    ("Vermelho", Color::Red),
    ("Verde", Color::Green),
    ("Azul", Color::Blue),
    ("Amarelo", Color::Yellow),
    ("Ciano", Color::Cyan),
    ("Magenta", Color::Magenta),
    ("Branco", Color::White),
];

/// Divide a área do terminal em regiões proporcionais à posição/tamanho real
/// de cada tela (via `xrandr`) — empilhado se todas tiverem o mesmo X (uma
/// embaixo da outra), lado a lado se todas tiverem o mesmo Y. Layout misto
/// (grid 2D de verdade) não é suportado — cai pra empilhado por ordem, é
/// aproximação, não perfeito.
fn layout_for_displays<'a>(
    area: Rect,
    displays: &'a [crate::hardware::video::Display],
) -> Vec<(Rect, &'a crate::hardware::video::Display)> {
    if displays.len() <= 1 {
        return displays.iter().map(|d| (area, d)).collect();
    }

    let same_x = displays.iter().all(|d| d.x == displays[0].x);

    let mut sorted: Vec<&crate::hardware::video::Display> = displays.iter().collect();
    if same_x {
        sorted.sort_by_key(|d| d.y);
    } else {
        sorted.sort_by_key(|d| d.x);
    }

    let total: u32 = sorted.iter().map(|d| if same_x { d.height } else { d.width }).sum();
    let constraints: Vec<Constraint> = sorted
        .iter()
        .map(|d| {
            let share = if same_x { d.height } else { d.width };
            Constraint::Ratio(share.max(1), total.max(1))
        })
        .collect();

    let direction = if same_x { Direction::Vertical } else { Direction::Horizontal };
    let regions = Layout::default().direction(direction).constraints(constraints).split(area);

    regions.iter().copied().zip(sorted.into_iter()).collect()
}

/// Preenche cada tela conectada com uma cor por vez, com pausa entre trocas
/// (não é pra piscar rápido tipo estroboscópio). Cada tela mostra só a
/// própria info (nome + resolução), não a de todas juntas. Esc/q sai a
/// qualquer momento, mesmo no meio da pausa de uma cor.
pub fn run_video_test(displays: &[crate::hardware::video::Display], hold: Duration) -> Result<()> {
    let mut screen = enter_screen()?;

    let result: Result<()> = 'outer: loop {
        for (color_name, color) in VIDEO_COLORS.iter() {
            if let Err(e) = screen.draw(|f| {
                for (region, display) in layout_for_displays(f.area(), displays) {
                    let text = format!(
                        "{}\n{}x{}\n\n{}\n\nEsc para sair",
                        display.name, display.width, display.height, color_name
                    );
                    let block = Paragraph::new(text)
                        .style(Style::default().fg(Color::Black).bg(*color))
                        .alignment(Alignment::Center);
                    f.render_widget(block, region);
                }
            }) {
                break 'outer Err(e.into());
            }

            let deadline = std::time::Instant::now() + hold;
            loop {
                let remaining = deadline.saturating_duration_since(std::time::Instant::now());
                if remaining.is_zero() {
                    break;
                }
                match poll_key(remaining.min(Duration::from_millis(100))) {
                    Ok(Some(KeyCode::Esc)) => break 'outer Ok(()),
                    Ok(_) => {}
                    Err(e) => break 'outer Err(e),
                }
            }
        }
    };

    leave_screen(screen)?;
    result
}

/// Mostra cru cada linha recebida em `rx` (ex: eventos do buttonhub), com
/// histórico das últimas 20. Essa tela existe pra testar CADA tecla física,
/// incluindo a que também serve de "sair" (`ke`/Esc) — se ela saísse no
/// primeiro toque o operador nunca conseguiria confirmar que ela funciona.
/// Por isso Esc exige dois toques seguidos (dentro de `ESC_EXIT_WINDOW`) pra
/// sair de fato; o primeiro só fica registrado no histórico normalmente e
/// mostra o aviso "aperte de novo pra sair". Qualquer outra tecla no meio
/// cancela essa confirmação pendente.
const ESC_EXIT_WINDOW: Duration = Duration::from_millis(1200);

pub fn run_event_stream(title: &str, rx: &std::sync::mpsc::Receiver<String>) -> Result<()> {
    let mut screen = enter_screen()?;
    let mut history: Vec<String> = Vec::new();
    let mut esc_pending_since: Option<std::time::Instant> = None;

    let result: Result<()> = loop {
        while let Ok(line) = rx.try_recv() {
            history.push(line);
            if history.len() > 20 {
                history.remove(0);
            }
        }

        if esc_pending_since.is_some_and(|t| t.elapsed() > ESC_EXIT_WINDOW) {
            esc_pending_since = None;
        }

        let hint = if esc_pending_since.is_some() {
            "Esc detectado — aperte de novo pra sair"
        } else {
            "Esc duas vezes seguidas pra sair (1a só testa a tecla)"
        };
        let mut lines = vec![format!("{} — {}", title, hint), String::new()];
        lines.extend(history.iter().cloned());
        if let Err(e) = draw_lines(&mut screen, title, &lines) {
            break Err(e);
        }

        match poll_key(Duration::from_millis(100)) {
            Ok(Some(KeyCode::Esc)) => {
                if esc_pending_since.is_some() {
                    break Ok(());
                }
                esc_pending_since = Some(std::time::Instant::now());
            }
            Ok(Some(_)) => esc_pending_since = None,
            Ok(None) => {}
            Err(e) => break Err(e),
        }
    };

    leave_screen(screen)?;
    result
}

/// Tela de espaço reservado — usada até cada teste real existir.
pub fn show_placeholder(title: &str, message: &str) -> Result<()> {
    with_screen(|terminal| {
        loop {
            terminal.draw(|f| {
                let block = Block::default()
                    .title(format!(" {} ", title))
                    .borders(Borders::ALL);
                let text = format!("{}\n\nAperte qualquer tecla pra voltar", message);
                f.render_widget(Paragraph::new(text).block(block), f.area());
            })?;

            if poll_key(Duration::from_millis(200))?.is_some() {
                break;
            }
        }
        Ok(())
    })
}
