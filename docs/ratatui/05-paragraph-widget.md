# Ratatui Paragraph Widget - Complete Guide

## Overview

The `Paragraph` widget is one of the most versatile widgets in Ratatui, designed for displaying multi-line text with automatic wrapping, scrolling, and alignment. Many developers manually split text or implement custom scrolling when `Paragraph` handles these features elegantly out of the box.

## Basic Usage

```rust
use ratatui::{
    widgets::Paragraph,
    text::Text,
    Frame,
};

fn render(frame: &mut Frame, area: Rect) {
    let text = "Hello, world!\nThis is a paragraph.";
    let paragraph = Paragraph::new(text);
    frame.render_widget(paragraph, area);
}
```

## Text Wrapping

### Wrap Configuration

The `Wrap` struct controls how text wraps when it exceeds the widget's width:

```rust
use ratatui::widgets::Wrap;

pub struct Wrap {
    pub trim: bool,  // Whether to trim leading whitespace on wrapped lines
}
```

### Wrapping Modes

```rust
use ratatui::widgets::{Paragraph, Wrap};

// No wrapping (default) - text gets cut off
let paragraph = Paragraph::new("Very long text that will be cut off");

// Word wrapping - breaks at word boundaries
let paragraph = Paragraph::new("Very long text that will wrap at word boundaries")
    .wrap(Wrap { trim: false });

// Word wrapping with trimming - removes leading whitespace
let paragraph = Paragraph::new("Very long text with    extra spaces")
    .wrap(Wrap { trim: true });
```

### Trim Behavior

The `trim` flag controls whitespace handling on wrapped lines:

```rust
// trim: false - preserves indentation
let text = "First line\n    Indented line\n        More indented";
Paragraph::new(text).wrap(Wrap { trim: false });
// Output:
// First line
//     Indented line
//         More indented

// trim: true - removes leading whitespace on wrapped lines
let text = "    This is a very long line that will wrap to the next line";
Paragraph::new(text).wrap(Wrap { trim: true });
// Output:
//     This is a very
// long line that will
// wrap to the next line
```

### Real-World Wrapping Examples

```rust
// Documentation display
fn render_docs(frame: &mut Frame, area: Rect, doc: &str) {
    let paragraph = Paragraph::new(doc)
        .wrap(Wrap { trim: true })  // Clean wrapping for docs
        .block(Block::bordered().title("Documentation"));
    frame.render_widget(paragraph, area);
}

// Log viewer (preserve formatting)
fn render_logs(frame: &mut Frame, area: Rect, logs: &str) {
    let paragraph = Paragraph::new(logs)
        .wrap(Wrap { trim: false });  // Keep indentation
    frame.render_widget(paragraph, area);
}

// Chat messages
fn render_message(frame: &mut Frame, area: Rect, message: &str) {
    let paragraph = Paragraph::new(message)
        .wrap(Wrap { trim: true })  // Clean word wrapping
        .alignment(Alignment::Left);
    frame.render_widget(paragraph, area);
}
```

## Scrolling

### Basic Scrolling

The `scroll()` method sets the scroll offset as `(vertical, horizontal)`:

```rust
use ratatui::widgets::Paragraph;

struct App {
    vertical_scroll: u16,
    horizontal_scroll: u16,
    content: String,
}

impl App {
    fn render(&self, frame: &mut Frame, area: Rect) {
        let paragraph = Paragraph::new(self.content.as_str())
            .scroll((self.vertical_scroll, self.horizontal_scroll))
            .wrap(Wrap { trim: true });
        
        frame.render_widget(paragraph, area);
    }
    
    fn scroll_down(&mut self) {
        self.vertical_scroll = self.vertical_scroll.saturating_add(1);
    }
    
    fn scroll_up(&mut self) {
        self.vertical_scroll = self.vertical_scroll.saturating_sub(1);
    }
    
    fn scroll_right(&mut self) {
        self.horizontal_scroll = self.horizontal_scroll.saturating_add(1);
    }
    
    fn scroll_left(&mut self) {
        self.horizontal_scroll = self.horizontal_scroll.saturating_sub(1);
    }
}
```

### Important: Scroll Order

**Note:** The scroll tuple is `(y, x)` (vertical, horizontal), which is **different** from the general `(x, y)` convention used elsewhere in Ratatui.

```rust
// Correct
paragraph.scroll((vertical, horizontal));
paragraph.scroll((10, 5));  // 10 lines down, 5 chars right

// Common mistake - don't confuse with (x, y) ordering
// This is (y, x) not (x, y)!
```

### Scroll with Bounds Checking

```rust
struct TextViewer {
    content: Vec<String>,
    vertical_scroll: usize,
    viewport_height: u16,
}

impl TextViewer {
    fn new(content: Vec<String>) -> Self {
        Self {
            content,
            vertical_scroll: 0,
            viewport_height: 0,
        }
    }
    
    fn scroll_down(&mut self) {
        let max_scroll = self.content.len().saturating_sub(self.viewport_height as usize);
        if self.vertical_scroll < max_scroll {
            self.vertical_scroll += 1;
        }
    }
    
    fn scroll_up(&mut self) {
        self.vertical_scroll = self.vertical_scroll.saturating_sub(1);
    }
    
    fn scroll_to_bottom(&mut self) {
        let max_scroll = self.content.len().saturating_sub(self.viewport_height as usize);
        self.vertical_scroll = max_scroll;
    }
    
    fn scroll_to_top(&mut self) {
        self.vertical_scroll = 0;
    }
    
    fn render(&mut self, frame: &mut Frame, area: Rect) {
        // Update viewport height
        self.viewport_height = area.height.saturating_sub(2); // Account for borders
        
        let text = self.content.join("\n");
        let paragraph = Paragraph::new(text)
            .block(Block::bordered().title("Text Viewer"))
            .scroll((self.vertical_scroll as u16, 0))
            .wrap(Wrap { trim: false });
        
        frame.render_widget(paragraph, area);
    }
}
```

### Page Scrolling

```rust
impl TextViewer {
    fn page_down(&mut self) {
        let page_size = self.viewport_height.saturating_sub(1); // Keep one line overlap
        self.vertical_scroll = self.vertical_scroll.saturating_add(page_size as usize);
        
        // Clamp to max
        let max_scroll = self.content.len().saturating_sub(self.viewport_height as usize);
        self.vertical_scroll = self.vertical_scroll.min(max_scroll);
    }
    
    fn page_up(&mut self) {
        let page_size = self.viewport_height.saturating_sub(1);
        self.vertical_scroll = self.vertical_scroll.saturating_sub(page_size as usize);
    }
}

// In event handler
match key.code {
    KeyCode::PageDown => app.viewer.page_down(),
    KeyCode::PageUp => app.viewer.page_up(),
    KeyCode::Home => app.viewer.scroll_to_top(),
    KeyCode::End => app.viewer.scroll_to_bottom(),
    _ => {}
}
```

### Auto-scroll (Following Content)

For log viewers or chat applications:

```rust
struct LogViewer {
    logs: Vec<String>,
    vertical_scroll: usize,
    auto_scroll: bool,
}

impl LogViewer {
    fn add_log(&mut self, log: String) {
        self.logs.push(log);
        
        // Auto-scroll to bottom if enabled
        if self.auto_scroll {
            self.scroll_to_bottom();
        }
    }
    
    fn scroll_to_bottom(&mut self) {
        // Will be clamped during render
        self.vertical_scroll = self.logs.len();
    }
    
    fn render(&mut self, frame: &mut Frame, area: Rect) {
        let viewport_height = area.height.saturating_sub(2) as usize;
        
        // Clamp scroll to valid range
        let max_scroll = self.logs.len().saturating_sub(viewport_height);
        self.vertical_scroll = self.vertical_scroll.min(max_scroll);
        
        let text = self.logs.join("\n");
        let paragraph = Paragraph::new(text)
            .block(Block::bordered().title(
                if self.auto_scroll { "Logs [Auto-scroll: ON]" } else { "Logs" }
            ))
            .scroll((self.vertical_scroll as u16, 0))
            .wrap(Wrap { trim: false });
        
        frame.render_widget(paragraph, area);
    }
}

// Toggle auto-scroll on user interaction
match key.code {
    KeyCode::Up | KeyCode::Down => {
        // Disable auto-scroll when user scrolls manually
        app.log_viewer.auto_scroll = false;
    }
    KeyCode::Char('a') => {
        // Toggle auto-scroll
        app.log_viewer.auto_scroll = !app.log_viewer.auto_scroll;
        if app.log_viewer.auto_scroll {
            app.log_viewer.scroll_to_bottom();
        }
    }
    _ => {}
}
```

## Alignment

Paragraph supports three alignment modes:

```rust
use ratatui::layout::Alignment;

// Left alignment (default)
let paragraph = Paragraph::new("Text")
    .alignment(Alignment::Left);

// Or use builder methods
let paragraph = Paragraph::new("Text").left_aligned();

// Center alignment
let paragraph = Paragraph::new("Centered Text")
    .alignment(Alignment::Center);
// Or
let paragraph = Paragraph::new("Centered Text").centered();

// Right alignment
let paragraph = Paragraph::new("Right-aligned")
    .alignment(Alignment::Right);
// Or
let paragraph = Paragraph::new("Right-aligned").right_aligned();
```

### Alignment with Wrapping

Alignment is applied **after** wrapping:

```rust
let text = "This is a very long line that will wrap to multiple lines when the widget width is exceeded";

// Center-aligned wrapped text
let paragraph = Paragraph::new(text)
    .centered()
    .wrap(Wrap { trim: true });

// Output (centered):
//    This is a very long
//   line that will wrap to
//  multiple lines when the
//    widget width is...
```

## Text Composition

Paragraph works with Ratatui's text types: `Text`, `Line`, and `Span`.

### Using Styled Text

```rust
use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
};

// Method 1: Vec of Lines
let lines = vec![
    Line::from("Header").bold().blue(),
    Line::from("Normal text"),
    Line::from(vec![
        Span::raw("Highlighted: "),
        Span::styled("Important", Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)),
    ]),
];
let paragraph = Paragraph::new(lines);

// Method 2: Text builder
let text = Text::from(vec![
    Line::from("First line"),
    Line::from("Second line").red(),
]);
let paragraph = Paragraph::new(text);

// Method 3: Simple string
let paragraph = Paragraph::new("Simple text");
```

### Dynamic Content

```rust
fn render_status(frame: &mut Frame, area: Rect, status: &AppStatus) {
    let lines = vec![
        Line::from(vec![
            Span::raw("Status: "),
            Span::styled(
                status.state.to_string(),
                match status.state {
                    State::Running => Style::default().fg(Color::Green),
                    State::Error => Style::default().fg(Color::Red),
                    State::Idle => Style::default().fg(Color::Gray),
                },
            ),
        ]),
        Line::from(format!("Items processed: {}", status.processed)),
        Line::from(format!("Errors: {}", status.errors)),
    ];
    
    let paragraph = Paragraph::new(lines)
        .block(Block::bordered().title("Status"));
    
    frame.render_widget(paragraph, area);
}
```

## Paragraph with Scrollbar

Combine Paragraph with Scrollbar for a complete scrollable text viewer:

```rust
use ratatui::{
    layout::Margin,
    widgets::{Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState, Wrap},
};

struct ScrollableText {
    content: Vec<String>,
    vertical_scroll: usize,
}

impl ScrollableText {
    fn render(&mut self, frame: &mut Frame, area: Rect) {
        let text = self.content.join("\n");
        let paragraph = Paragraph::new(text)
            .block(Block::bordered().title("Scrollable Text"))
            .scroll((self.vertical_scroll as u16, 0))
            .wrap(Wrap { trim: false });
        
        frame.render_widget(paragraph, area);
        
        // Render scrollbar
        let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
            .begin_symbol(Some("↑"))
            .end_symbol(Some("↓"));
        
        let mut scrollbar_state = ScrollbarState::new(self.content.len())
            .position(self.vertical_scroll);
        
        frame.render_stateful_widget(
            scrollbar,
            area.inner(Margin {
                vertical: 1,
                horizontal: 0,
            }),
            &mut scrollbar_state,
        );
    }
}
```

## Styling

### Widget-level Styling

```rust
// Style applies to entire widget (text + block)
let paragraph = Paragraph::new("Text")
    .style(Style::default().fg(Color::White).bg(Color::Black));

// Or use shorthand methods
let paragraph = Paragraph::new("Text")
    .white()
    .on_black();
```

### Block Styling

```rust
let paragraph = Paragraph::new("Content")
    .block(
        Block::bordered()
            .title("Title")
            .style(Style::default().fg(Color::Cyan))
    )
    .style(Style::default().fg(Color::White));  // Text style
```

### Style Precedence

Styles are combined in this order:
1. Widget style (applied to everything)
2. Block style (if present)
3. Individual text styles (Span/Line styles)

```rust
let text = vec![
    Line::from("Default color"),
    Line::from("Red").red(),  // Overrides widget style
];

let paragraph = Paragraph::new(text)
    .style(Style::default().fg(Color::White))  // Default white
    .block(Block::bordered());  // Block inherits white

// Result:
// - Block borders: white
// - "Default color": white
// - "Red": red (span style overrides)
```

## Advanced Patterns

### Multi-column Text Display

```rust
struct MultiColumnText {
    columns: Vec<Vec<String>>,
    scroll: usize,
}

impl MultiColumnText {
    fn render(&self, frame: &mut Frame, area: Rect) {
        let chunks = Layout::horizontal(
            std::iter::repeat(Constraint::Fill(1))
                .take(self.columns.len())
                .collect::<Vec<_>>()
        ).split(area);
        
        for (i, column) in self.columns.iter().enumerate() {
            let text = column.join("\n");
            let paragraph = Paragraph::new(text)
                .scroll((self.scroll as u16, 0))
                .wrap(Wrap { trim: true })
                .block(Block::bordered());
            
            frame.render_widget(paragraph, chunks[i]);
        }
    }
}
```

### Highlighted Search Results

```rust
fn render_with_highlights(
    frame: &mut Frame,
    area: Rect,
    content: &str,
    search: &str,
) {
    let lines: Vec<Line> = content
        .lines()
        .map(|line| {
            if search.is_empty() {
                return Line::from(line.to_string());
            }
            
            let mut spans = Vec::new();
            let mut last_end = 0;
            
            // Find all occurrences of search term
            for (idx, _) in line.match_indices(search) {
                // Text before match
                if idx > last_end {
                    spans.push(Span::raw(line[last_end..idx].to_string()));
                }
                // Highlighted match
                spans.push(
                    Span::styled(
                        search.to_string(),
                        Style::default().bg(Color::Yellow).fg(Color::Black),
                    )
                );
                last_end = idx + search.len();
            }
            
            // Remaining text
            if last_end < line.len() {
                spans.push(Span::raw(line[last_end..].to_string()));
            }
            
            Line::from(spans)
        })
        .collect();
    
    let paragraph = Paragraph::new(lines)
        .wrap(Wrap { trim: true })
        .block(Block::bordered().title("Search Results"));
    
    frame.render_widget(paragraph, area);
}
```

### Line Numbers

```rust
fn render_with_line_numbers(
    frame: &mut Frame,
    area: Rect,
    lines: &[String],
    scroll: usize,
) {
    let horizontal = Layout::horizontal([
        Constraint::Length(6),  // Line numbers
        Constraint::Min(0),     // Content
    ]);
    let [numbers_area, content_area] = horizontal.areas(area);
    
    // Line numbers
    let line_numbers: Vec<String> = (scroll + 1..=scroll + lines.len())
        .map(|n| format!("{:>4} │", n))
        .collect();
    
    let numbers = Paragraph::new(line_numbers.join("\n"))
        .style(Style::default().fg(Color::DarkGray));
    
    frame.render_widget(numbers, numbers_area);
    
    // Content
    let content = Paragraph::new(lines.join("\n"))
        .scroll((scroll as u16, 0))
        .wrap(Wrap { trim: false });
    
    frame.render_widget(content, content_area);
}
```

## Common Pitfalls

### ❌ Not Using Wrap

```rust
// Wrong - long lines get cut off
let paragraph = Paragraph::new("This is a very long line that will definitely exceed the widget width");
```

```rust
// Correct - text wraps naturally
let paragraph = Paragraph::new("This is a very long line that will definitely exceed the widget width")
    .wrap(Wrap { trim: true });
```

### ❌ Scroll Coordinate Confusion

```rust
// Wrong - (x, y) order
paragraph.scroll((horizontal, vertical));
```

```rust
// Correct - (y, x) order
paragraph.scroll((vertical, horizontal));
```

### ❌ Not Clamping Scroll

```rust
// Wrong - can scroll beyond content
app.scroll += 1;
```

```rust
// Correct - clamp to content bounds
let max_scroll = app.lines.len().saturating_sub(viewport_height);
app.scroll = (app.scroll + 1).min(max_scroll);
```

### ❌ Using Paragraph When Not Needed

```rust
// Overkill - no wrapping or blocks needed
let paragraph = Paragraph::new("Simple text");
```

```rust
// Better - render text directly
let line = Line::from("Simple text");
frame.render_widget(line, area);
```

## Performance Considerations

- **Wrapping is computed each render**: For large texts, this can be expensive
- **Consider chunking large documents**: Only render visible portions
- **Reuse Text objects**: Build text once, not on every frame
- **Avoid recreating styled content**: Cache styled Lines/Spans when possible

## Limitations and Workarounds

### Getting Wrapped Line Count

Paragraph doesn't expose how many lines the wrapped content will occupy. Workaround:

```rust
// Estimate wrapped lines (rough approximation)
fn estimate_wrapped_lines(text: &str, width: u16) -> usize {
    text.lines()
        .map(|line| {
            let line_len = line.len() as u16;
            (line_len + width - 1) / width  // Ceiling division
        })
        .sum::<u16>() as usize
}
```

### Horizontal Scroll with Wrapping

Horizontal scrolling has limited use with wrapping enabled, as wrapped lines reset horizontal position.

### Bi-directional Scroll

For complex bi-directional scrolling with zoom, consider using the `tui-scrollview` crate.

## Testing

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{backend::TestBackend, Terminal};
    
    #[test]
    fn test_paragraph_render() {
        let backend = TestBackend::new(40, 10);
        let mut terminal = Terminal::new(backend).unwrap();
        
        let paragraph = Paragraph::new("Test content")
            .wrap(Wrap { trim: true });
        
        terminal.draw(|frame| {
            frame.render_widget(paragraph, frame.area());
        }).unwrap();
        
        // Verify rendering
        let buffer = terminal.backend().buffer();
        assert_eq!(buffer.get(0, 0).symbol(), "T");
    }
}
```

## When to Use Paragraph

**Use Paragraph when:**
- Displaying multi-line text that may wrap
- Need scrolling for long content
- Want to apply alignment to text blocks
- Text is surrounded by a block/border

**Don't use Paragraph when:**
- Displaying a single short line (use `Line` directly)
- Building tables (use `Table` widget)
- Building selectable lists (use `List` widget)
- Need complex layouts (use `Layout`)

## Further Reading

- [Paragraph API Docs](https://docs.rs/ratatui/latest/ratatui/widgets/struct.Paragraph.html)
- [Wrap API Docs](https://docs.rs/ratatui/latest/ratatui/widgets/struct.Wrap.html)
- [Text Composition Guide](https://ratatui.rs/concepts/text-and-styling/)
- [Paragraph Example](https://ratatui.rs/examples/widgets/paragraph/)
- [Scrollbar Widget](https://docs.rs/ratatui/latest/ratatui/widgets/struct.Scrollbar.html)
