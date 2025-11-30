# Ratatui Text and Span Composition - Complete Guide

## Overview

Text rendering in Ratatui follows a hierarchical model: **Text** → **Line** → **Span**. Understanding this composition system and its natural conversions can eliminate manual string formatting and make styling far more elegant. Many developers concatenate strings with escape codes when Ratatui's text types handle this natively.

## The Text Hierarchy

```
Text (multi-line container)
 ├── Line (single line)
 │    ├── Span (styled fragment)
 │    ├── Span (styled fragment)
 │    └── Span (styled fragment)
 ├── Line (single line)
 │    └── Span (styled fragment)
 └── ...
```

## Span - The Atomic Unit

A `Span` is the most basic unit: a piece of text with a single style.

### Creating Spans

```rust
use ratatui::{
    text::Span,
    style::{Color, Modifier, Style, Stylize},
};

// Raw span (no styling)
let span = Span::raw("Hello");

// Styled span (explicit Style)
let span = Span::styled(
    "Hello",
    Style::default()
        .fg(Color::Red)
        .bg(Color::Yellow)
        .add_modifier(Modifier::BOLD)
);

// Using Stylize trait (shorthand - recommended!)
let span = "Hello".red().bold();

// From implementations (automatic conversion)
let span: Span = "Hello".into();
let span = Span::from("Hello");
```

### Stylize Trait Magic

The `Stylize` trait provides builder-style methods for quick styling:

```rust
use ratatui::style::Stylize;

// Color methods
"text".black();
"text".red();
"text".green();
"text".yellow();
"text".blue();
"text".magenta();
"text".cyan();
"text".white();
"text".gray();
"text".dark_gray();
"text".light_red();
"text".light_green();
// ... and more

// Background color methods
"text".on_black();
"text".on_red();
"text".on_green();
// ... and more

// Modifier methods
"text".bold();
"text".italic();
"text".underlined();
"text".crossed_out();
"text".dim();
"text".slow_blink();
"text".rapid_blink();
"text".reversed();
"text".hidden();

// Chaining
"text".red().bold().on_white().italic();

// RGB colors
"text".fg(Color::Rgb(255, 128, 64));
"text".bg(Color::Rgb(32, 32, 32));
```

### Span Methods

```rust
let mut span = Span::raw("Hello");

// Set content
span = span.content("New content");

// Set style
span = span.style(Style::default().fg(Color::Blue));

// Patch style (add to existing)
span = span.patch_style(Style::default().add_modifier(Modifier::BOLD));

// Reset style
span = span.reset_style();

// Get width (unicode-aware)
let width = span.width();  // Returns number of display columns

// Iterate over graphemes
for grapheme in span.styled_graphemes(Style::default()) {
    // Process each grapheme with its style
}
```

## Line - Single Line Composition

A `Line` represents a single line of text composed of multiple `Span`s.

### Creating Lines

```rust
use ratatui::text::Line;

// From a string
let line = Line::raw("Hello");
let line = Line::from("Hello");

// From styled string
let line = Line::styled("Hello", Style::default().fg(Color::Green));

// From Vec<Span>
let line = Line::from(vec![
    Span::raw("Hello "),
    Span::styled("World", Style::default().fg(Color::Red)),
]);

// Using Stylize on strings (auto-conversion)
let line: Line = vec![
    "Hello ".into(),
    "World".red(),
].into();

// More concise with Into trait
let line = Line::from(vec![
    "Status: ",
    "OK".green(),
    "!".bold(),
]);
```

### Important: Newline Handling

**Key behavior:** When you create a Line from text containing `\n`, it **splits into multiple Spans**:

```rust
let line = Line::raw("Hello\nWorld");

// Results in:
// Line {
//     spans: [
//         Span { content: "Hello" },
//         Span { content: "World" },
//     ]
// }

// The line is NOT split into multiple lines,
// just multiple spans within one line
```

### Line Alignment

Lines can have their own alignment:

```rust
// Left alignment (default)
let line = Line::from("Text").alignment(Alignment::Left);
let line = Line::from("Text").left_aligned();

// Center alignment
let line = Line::from("Title").centered();

// Right alignment
let line = Line::from("Text").right_aligned();
```

### Line Styling

```rust
// Style the entire line
let line = Line::from(vec![
    "Hello ",
    "World".red(),  // This red overrides line style
])
.style(Style::default().fg(Color::Blue));

// Result: "Hello " is blue, "World" is red
```

### Line Methods

```rust
let mut line = Line::from("Hello");

// Add spans
line.push_span(Span::raw(" World"));
line.push_span("!".bold());

// Iterate spans
for span in line.spans {
    // Process each span
}

// Width calculation
let width = line.width();

// Style entire line
line = line.style(Style::default().fg(Color::Yellow));

// Patch style
line = line.patch_style(Style::default().bold());
```

## Text - Multi-line Container

`Text` is the top-level container holding multiple `Line`s.

### Creating Text

```rust
use ratatui::text::Text;

// From a string (handles \n automatically)
let text = Text::raw("Line 1\nLine 2\nLine 3");

// From styled string
let text = Text::styled(
    "Line 1\nLine 2",
    Style::default().fg(Color::Green)
);

// From Vec<Line>
let text = Text::from(vec![
    Line::from("First line"),
    Line::from("Second line").red(),
    Line::from(vec![
        "Third ".into(),
        "line".bold(),
    ]),
]);

// Using Stylize
let text = Text::from(vec![
    "Header".blue().bold(),
    "Content line 1",
    "Content line 2",
    "Footer".gray().italic(),
]);
```

### Building Text Dynamically

```rust
// Start with empty text
let mut text = Text::default();

// Add lines
text.lines.push(Line::from("First line"));
text.lines.push("Second line".red());

// Or use extend
text.extend(vec![
    Line::from("Third"),
    Line::from("Fourth"),
]);

// Push span to last line
text.push_span(" - appended to last line");

// Concatenate Text instances
let text1 = Text::from("Part 1");
let text2 = Text::from("Part 2");
let combined = text1 + text2;

// Or use +=
let mut text = Text::from("Start");
text += Text::from("More content");
```

### Text Styling and Alignment

```rust
let mut text = Text::from(vec![
    Line::from("Line 1"),
    Line::from("Line 2"),
]);

// Style all lines
text = text.style(Style::default().fg(Color::White));

// Set alignment for all lines (unless line has its own)
text = text.alignment(Alignment::Center);

// Patch style
text = text.patch_style(Style::default().bold());

// Reset style
text = text.reset_style();
```

## Practical Composition Patterns

### Status Messages

```rust
fn status_message(status: &str, is_ok: bool) -> Line<'static> {
    Line::from(vec![
        Span::raw("Status: "),
        Span::styled(
            status,
            if is_ok {
                Style::default().fg(Color::Green)
            } else {
                Style::default().fg(Color::Red).bold()
            }
        ),
    ])
}

// Usage
let line = status_message("Connected", true);
let line = status_message("Error", false);
```

### Highlighted Keywords

```rust
fn highlight_keywords(text: &str, keywords: &[&str]) -> Line {
    let mut spans = Vec::new();
    let mut remaining = text;
    
    while !remaining.is_empty() {
        // Find next keyword
        let mut next_match = None;
        for keyword in keywords {
            if let Some(idx) = remaining.find(keyword) {
                if next_match.is_none() || idx < next_match.unwrap().0 {
                    next_match = Some((idx, keyword));
                }
            }
        }
        
        match next_match {
            Some((idx, keyword)) => {
                // Text before keyword
                if idx > 0 {
                    spans.push(Span::raw(remaining[..idx].to_string()));
                }
                // Highlighted keyword
                spans.push(Span::styled(
                    keyword.to_string(),
                    Style::default().fg(Color::Yellow).bold(),
                ));
                remaining = &remaining[idx + keyword.len()..];
            }
            None => {
                // No more keywords
                spans.push(Span::raw(remaining.to_string()));
                break;
            }
        }
    }
    
    Line::from(spans)
}

// Usage
let line = highlight_keywords("fn main() { println!(\"Hello\"); }", &["fn", "println"]);
```

### Progress Indicators

```rust
fn progress_line(current: usize, total: usize) -> Line<'static> {
    let percentage = (current as f64 / total as f64 * 100.0) as u8;
    Line::from(vec![
        "Progress: ".into(),
        format!("{}/{}", current, total).bold(),
        " (".into(),
        format!("{}%", percentage).cyan(),
        ")".into(),
    ])
}
```

### Log Levels

```rust
enum LogLevel {
    Info,
    Warning,
    Error,
}

fn log_line(level: LogLevel, message: &str) -> Line<'static> {
    let (level_str, style) = match level {
        LogLevel::Info => ("INFO", Style::default().fg(Color::Cyan)),
        LogLevel::Warning => ("WARN", Style::default().fg(Color::Yellow)),
        LogLevel::Error => ("ERROR", Style::default().fg(Color::Red).bold()),
    };
    
    Line::from(vec![
        Span::raw("["),
        Span::styled(level_str, style),
        Span::raw("] "),
        Span::raw(message.to_string()),
    ])
}
```

### Timestamps

```rust
use chrono::Local;

fn timestamped_line(message: &str) -> Line<'static> {
    let timestamp = Local::now().format("%H:%M:%S");
    Line::from(vec![
        format!("[{}] ", timestamp).dark_gray(),
        message.into(),
    ])
}
```

### Tables in Text

```rust
fn table_lines(headers: &[&str], rows: &[Vec<String>]) -> Text<'static> {
    let mut lines = vec![
        // Header
        Line::from(
            headers
                .iter()
                .map(|h| Span::styled(*h, Style::default().bold().fg(Color::Cyan)))
                .collect::<Vec<_>>()
        ),
        // Separator
        Line::from("─".repeat(headers.len() * 15).dark_gray()),
    ];
    
    // Rows
    for row in rows {
        lines.push(Line::from(
            row.iter()
                .map(|cell| Span::raw(format!("{:<15}", cell)))
                .collect::<Vec<_>>()
        ));
    }
    
    Text::from(lines)
}
```

## Style Inheritance and Composition

Styles flow down the hierarchy and are combined using `patch()`:

```rust
// Text-level style
let mut text = Text::from(vec![
    Line::from("Default color"),  // Inherits text style
    Line::from("Red").red(),        // Overrides with red
]);
text = text.style(Style::default().fg(Color::White));

// Line-level style
let line = Line::from(vec![
    "Default ",                     // Inherits line style
    "Bold".bold(),                  // Adds bold to line style
])
.style(Style::default().fg(Color::Blue));

// Result:
// - "Default " is blue
// - "Bold" is blue and bold
```

### patch_style vs style

```rust
let span = "Text"
    .fg(Color::Red)                        // Set red
    .patch_style(Style::default().bold()); // Add bold
// Result: red + bold

let span = "Text"
    .fg(Color::Red)                        // Set red
    .style(Style::default().bold());       // Replace with just bold
// Result: just bold (red lost)
```

## Conversions and Type Inference

Ratatui provides extensive `From` implementations for natural conversions:

### String-like to Span

```rust
let span: Span = "text".into();
let span: Span = String::from("text").into();
let span: Span = Cow::Borrowed("text").into();
```

### Span to Line

```rust
let line: Line = Span::raw("text").into();
let line: Line = "text".into();
```

### Vec<Span> to Line

```rust
let line: Line = vec![
    "Hello ".into(),
    "World".red(),
].into();
```

### Line to Text

```rust
let text: Text = Line::from("text").into();
```

### Vec<Line> to Text

```rust
let text: Text = vec![
    Line::from("Line 1"),
    Line::from("Line 2"),
].into();
```

### Automatic Conversions in Context

```rust
// Paragraph accepts many types
Paragraph::new("string");                          // &str
Paragraph::new(String::from("text"));              // String
Paragraph::new(vec!["line1", "line2"]);            // Vec<&str>
Paragraph::new(vec![Line::from("line1")]);         // Vec<Line>
Paragraph::new(Text::from("text"));                // Text

// Block titles accept strings or styled text
Block::new().title("Simple title");
Block::new().title(Span::styled("Styled", Style::default().bold()));
Block::new().title(vec!["Multi", " part".bold()]);
```

## Common Patterns

### Building Dynamic Content

```rust
fn render_stats(frame: &mut Frame, area: Rect, stats: &Stats) {
    let lines = vec![
        Line::from(vec![
            "CPU: ".into(),
            format!("{}%", stats.cpu).cyan().bold(),
        ]),
        Line::from(vec![
            "Memory: ".into(),
            format!("{}/{} MB", stats.mem_used, stats.mem_total)
                .if_then_else(
                    stats.mem_used > stats.mem_total * 80 / 100,
                    |s| s.red(),
                    |s| s.green()
                ),
        ]),
        Line::from(vec![
            "Disk: ".into(),
            format!("{}%", stats.disk).yellow(),
        ]),
    ];
    
    let paragraph = Paragraph::new(lines)
        .block(Block::bordered().title("System Stats"));
    
    frame.render_widget(paragraph, area);
}
```

### Conditional Styling

```rust
fn styled_by_condition(text: &str, condition: bool) -> Span {
    if condition {
        text.green().bold()
    } else {
        text.red()
    }
}

// Or inline
let span = format!("Status: {}", status).fg(
    if status == "OK" { Color::Green } else { Color::Red }
);
```

### Text Builders

```rust
struct TextBuilder {
    lines: Vec<Line<'static>>,
}

impl TextBuilder {
    fn new() -> Self {
        Self { lines: Vec::new() }
    }
    
    fn add_line(&mut self, line: Line<'static>) -> &mut Self {
        self.lines.push(line);
        self
    }
    
    fn add_header(&mut self, text: &str) -> &mut Self {
        self.lines.push(Line::from(text).bold().blue());
        self
    }
    
    fn add_separator(&mut self) -> &mut Self {
        self.lines.push(Line::from("─".repeat(80).dark_gray()));
        self
    }
    
    fn build(self) -> Text<'static> {
        Text::from(self.lines)
    }
}

// Usage
let text = TextBuilder::new()
    .add_header("Report")
    .add_separator()
    .add_line(Line::from("Content here"))
    .build();
```

## Performance Tips

### Reuse Text Objects

```rust
// ❌ Recreating every frame
fn render(frame: &mut Frame, area: Rect) {
    let text = Text::from(vec![
        "Line 1".red(),
        "Line 2".green(),
        // ...
    ]);
    frame.render_widget(Paragraph::new(text), area);
}

// ✅ Create once, reuse
struct App {
    static_text: Text<'static>,
}

impl App {
    fn new() -> Self {
        Self {
            static_text: Text::from(vec![
                "Line 1".red(),
                "Line 2".green(),
            ]),
        }
    }
    
    fn render(&self, frame: &mut Frame, area: Rect) {
        frame.render_widget(
            Paragraph::new(self.static_text.clone()),
            area
        );
    }
}
```

### Use References When Possible

```rust
// Avoid unnecessary clones
let line = Line::from("Text");
frame.render_widget(&line, area);  // Implements WidgetRef
```

## Common Pitfalls

### ❌ Manual String Concatenation

```rust
// Wrong - hard to style, error-prone
let text = format!("Status: {} | Count: {}", 
    status, 
    count
);
let paragraph = Paragraph::new(text);
```

```rust
// Right - composable and stylable
let line = Line::from(vec![
    "Status: ".into(),
    status.green().bold(),
    " | Count: ".into(),
    count.to_string().cyan(),
]);
let paragraph = Paragraph::new(line);
```

### ❌ Not Using Into/From

```rust
// Verbose
let line = Line::from(vec![
    Span::raw("Hello "),
    Span::styled("World", Style::default().red()),
]);
```

```rust
// Concise
let line = Line::from(vec![
    "Hello ",
    "World".red(),
]);
```

### ❌ Forgetting Style Composition

```rust
// Wrong - overwrites red with just bold
let span = "Text".red().style(Style::default().bold());
```

```rust
// Right - combines red and bold
let span = "Text".red().bold();
// Or
let span = "Text".red().patch_style(Style::default().bold());
```

### ❌ Breaking Line Abstraction

```rust
// Wrong - treating Line as just text
let mut text = String::new();
for item in items {
    text.push_str(&format!("{}\n", item));
}
let paragraph = Paragraph::new(text);
```

```rust
// Right - use Line/Text properly
let lines: Vec<Line> = items
    .iter()
    .map(|item| Line::from(item.to_string()))
    .collect();
let paragraph = Paragraph::new(lines);
```

## Testing

```rust
#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_span_width() {
        assert_eq!(Span::raw("Hello").width(), 5);
        assert_eq!(Span::raw("世界").width(), 4);  // CJK characters
    }
    
    #[test]
    fn test_line_composition() {
        let line = Line::from(vec![
            "Hello ",
            "World".red(),
        ]);
        assert_eq!(line.spans.len(), 2);
        assert_eq!(line.width(), 11);
    }
    
    #[test]
    fn test_text_extension() {
        let mut text = Text::from("Line 1");
        text.lines.push("Line 2".into());
        assert_eq!(text.lines.len(), 2);
    }
}
```

## When to Use What

**Use Span when:**
- You need a single styled text fragment
- Building parts of a Line
- Implementing custom text rendering

**Use Line when:**
- Representing a single line with mixed styling
- Building rows for lists or tables
- Need per-line alignment

**Use Text when:**
- Multi-line content
- Need to apply style to multiple lines
- Building paragraphs or text blocks
- Passing to Paragraph or similar widgets

**Direct rendering:**
```rust
// For simple cases, render directly (no Paragraph needed)
frame.render_widget("Simple text", area);
frame.render_widget(Line::from("A line"), area);
frame.render_widget(Span::raw("A span"), area);
```

## Further Reading

- [Text Module API](https://docs.rs/ratatui/latest/ratatui/text/index.html)
- [Span API](https://docs.rs/ratatui/latest/ratatui/text/struct.Span.html)
- [Line API](https://docs.rs/ratatui/latest/ratatui/text/struct.Line.html)
- [Text API](https://docs.rs/ratatui/latest/ratatui/text/struct.Text.html)
- [Styling Text Guide](https://ratatui.rs/recipes/render/style-text/)
- [Displaying Text Guide](https://ratatui.rs/recipes/render/display-text/)
