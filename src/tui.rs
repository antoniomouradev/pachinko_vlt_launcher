use anyhow::Result;
use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    prelude::*,
    widgets::{Block, Borders, List, ListItem, Paragraph},
};
use std::io;
use std::time::Duration;

pub enum MenuChoice {
    TestMachine,
    ConfigureMachine,
    Shutdown,
}

pub enum TestChoice {
    Inputs,
    Audio,
    Video,
    Connection,
}

const MENU_ITEMS: [&str; 3] = ["Testar Máquina", "Configurar Máquina", "Desligar"];

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
/// Esc/q volta `None`. Navegação por teclado por enquanto — troca pra ler as
/// 8 teclas físicas da máquina fica pra quando o mapeamento de input real
/// (`/dev/input/event*`) entrar.
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

            if event::poll(Duration::from_millis(200))? {
                if let Event::Key(key) = event::read()? {
                    if key.kind == KeyEventKind::Press {
                        match key.code {
                            KeyCode::Up => selected = selected.saturating_sub(1),
                            KeyCode::Down => selected = (selected + 1).min(items.len() - 1),
                            KeyCode::Enter => break Some(selected),
                            KeyCode::Esc | KeyCode::Char('q') => break None,
                            _ => {}
                        }
                    }
                }
            }
        };

        Ok(choice)
    })
}

pub fn run_menu() -> Result<Option<MenuChoice>> {
    let choice = select(" Pachinko3 ", &MENU_ITEMS)?;
    Ok(choice.map(|i| match i {
        0 => MenuChoice::TestMachine,
        1 => MenuChoice::ConfigureMachine,
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
pub fn poll_key(timeout: Duration) -> Result<Option<KeyCode>> {
    if event::poll(timeout)? {
        if let Event::Key(key) = event::read()? {
            if key.kind == KeyEventKind::Press {
                return Ok(Some(key.code));
            }
        }
    }
    Ok(None)
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
                        "{}\n{}x{}\n\n{}\n\nEsc/q para sair",
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
                    Ok(Some(KeyCode::Esc)) | Ok(Some(KeyCode::Char('q'))) => break 'outer Ok(()),
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
/// histórico das últimas 20. Esc/q sai a qualquer momento.
pub fn run_event_stream(title: &str, rx: &std::sync::mpsc::Receiver<String>) -> Result<()> {
    let mut screen = enter_screen()?;
    let mut history: Vec<String> = Vec::new();

    let result: Result<()> = loop {
        while let Ok(line) = rx.try_recv() {
            history.push(line);
            if history.len() > 20 {
                history.remove(0);
            }
        }

        let mut lines = vec![format!("{} — Esc/q para sair", title), String::new()];
        lines.extend(history.iter().cloned());
        if let Err(e) = draw_lines(&mut screen, title, &lines) {
            break Err(e);
        }

        match poll_key(Duration::from_millis(100)) {
            Ok(Some(KeyCode::Esc)) | Ok(Some(KeyCode::Char('q'))) => break Ok(()),
            Ok(_) => {}
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
                let text = format!("{}\n\nPressione qualquer tecla para voltar", message);
                f.render_widget(Paragraph::new(text).block(block), f.area());
            })?;

            if event::poll(Duration::from_millis(200))? {
                if let Event::Key(key) = event::read()? {
                    if key.kind == KeyEventKind::Press {
                        break;
                    }
                }
            }
        }
        Ok(())
    })
}
