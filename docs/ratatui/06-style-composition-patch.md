# Ratatui Style Composition with patch() - Complete Guide

## Overview

One of the most powerful but commonly overlooked features in Ratatui is style composition using the `patch()` and `patch_style()` methods. Instead of repeatedly defining complete styles, you can build styles incrementally and combine them hierarchically. This leads to more maintainable, DRY (Don't Repeat Yourself) code and enables powerful theming systems.

## Understanding Style vs patch_style

### The Key Difference

```rust
// style() - REPLACES the entire style
let span = "text".red().style(Style::default().bold());
// Result: ONLY bold (red is LOST!)

// patch_style() - ADDS to existing style  
let span = "text".red().patch_style(Style::default().bold());
// Result: red AND bold (combined!)
```

### Why It Matters

```rust
// ❌ Wrong: Loses previous styling
fn highlight(span: Span) -> Span {
    span.style(Style::default().bold())  // Loses original colors!
}

// ✅ Right: Preserves and enhances
fn highlight(span: Span) -> Span {
    span.patch_style(Style::default().bold())  // Keeps colors, adds bold!
}
```

## How patch() Works

The `patch()` method on `Style` merges two styles together:

```rust
use ratatui::style::{Color, Modifier, Style};

let base = Style::default()
    .fg(Color::Blue)
    .add_modifier(Modifier::ITALIC);

let overlay = Style::default()
    .bg(Color::White)
    .add_modifier(Modifier::BOLD);

let combined = base.patch(overlay);
// Result: Blue text, white background, bold AND italic
```

### Merge Rules

- **Foreground color**: Overlay wins (if set)
- **Background color**: Overlay wins (if set)
- **Underline color**: Overlay wins (if set)
- **Add modifiers**: Combined (union)
- **Sub modifiers**: Combined (union)

```rust
let style1 = Style::default()
    .fg(Color::Red)
    .bg(Color::Black)
    .add_modifier(Modifier::BOLD);

let style2 = Style::default()
    .fg(Color::Blue)  // Will replace red
    .add_modifier(Modifier::ITALIC);  // Will add to bold

let result = style1.patch(style2);
// Result:
// - fg: Blue (replaced)
// - bg: Black (kept)
// - modifiers: BOLD | ITALIC (combined)
```

## Style Hierarchy in Widgets

Styles flow down through the widget hierarchy and are composed:

```rust
// Widget level
let paragraph = Paragraph::new("text")
    .style(Style::default().fg(Color::White));  // Base style

// Block level
let paragraph = paragraph
    .block(Block::bordered()
        .style(Style::default().bg(Color::Blue))  // Patches widget style
    );

// Text level
let text = Text::styled("Hello", Style::default().bold());  // Patches widget+block
let paragraph = Paragraph::new(text).style(Style::default().fg(Color::Red));

// Result: Red, bold text on blue background with white as fallback
```

## Building Theme Systems

### Basic Theme

```rust
use ratatui::style::{Color, Modifier, Style};

pub struct Theme {
    pub normal: Style,
    pub highlight: Style,
    pub error: Style,
    pub success: Style,
    pub warning: Style,
}

impl Theme {
    pub fn default() -> Self {
        let base = Style::default().bg(Color::Black);
        
        Self {
            normal: base.fg(Color::White),
            highlight: base.fg(Color::Cyan).add_modifier(Modifier::BOLD),
            error: base.fg(Color::Red).add_modifier(Modifier::BOLD),
            success: base.fg(Color::Green),
            warning: base.fg(Color::Yellow),
        }
    }
    
    pub fn dark() -> Self {
        let base = Style::default().bg(Color::Rgb(20, 20, 20));
        
        Self {
            normal: base.fg(Color::Rgb(200, 200, 200)),
            highlight: base.fg(Color::Rgb(100, 200, 255)).add_modifier(Modifier::BOLD),
            error: base.fg(Color::Rgb(255, 100, 100)).add_modifier(Modifier::BOLD),
            success: base.fg(Color::Rgb(100, 255, 100)),
            warning: base.fg(Color::Rgb(255, 200, 100)),
        }
    }
}

// Usage
impl App {
    fn render(&self, frame: &mut Frame, area: Rect) {
        let theme = Theme::default();
        
        let text = vec![
            Line::from("Normal text").style(theme.normal),
            Line::from("Highlighted").style(theme.highlight),
            Line::from("Error!").style(theme.error),
            Line::from("Success").style(theme.success),
        ];
        
        let paragraph = Paragraph::new(text);
        frame.render_widget(paragraph, area);
    }
}
```

### Advanced Theme with Composition

```rust
pub struct Theme {
    // Base styles
    pub base: Style,
    pub base_bold: Style,
    pub base_dim: Style,
    
    // Semantic colors
    pub primary: Color,
    pub secondary: Color,
    pub danger: Color,
    pub warning: Color,
    pub success: Color,
    pub info: Color,
    
    // Component styles
    pub header: Style,
    pub content: Style,
    pub footer: Style,
    pub selected: Style,
}

impl Theme {
    pub fn new(primary: Color, bg: Color) -> Self {
        let base = Style::default()
            .bg(bg)
            .fg(Color::White);
        
        Self {
            base,
            base_bold: base.add_modifier(Modifier::BOLD),
            base_dim: base.fg(Color::DarkGray),
            
            primary,
            secondary: Color::Cyan,
            danger: Color::Red,
            warning: Color::Yellow,
            success: Color::Green,
            info: Color::Blue,
            
            header: base.fg(primary).add_modifier(Modifier::BOLD),
            content: base,
            footer: base.fg(Color::DarkGray),
            selected: base.fg(primary).add_modifier(Mod(to.fg),
    };
    
    let bg = match (from.bg, to.bg) {
        (Some(from_bg), Some(to_bg)) => Some(interpolate_color(from_bg, to_bg, t)),
        _ => from.bg.or(to.bg),
    };
    
    Style::default()
        .fg(fg.unwrap_or(Color::Reset))
        .bg(bg.unwrap_or(Color::Reset))
}

// Animated transition
struct TransitionState {
    from: Style,
    to: Style,
    progress: f32,
}

impl TransitionState {
    fn current_style(&self) -> Style {
        interpolate_style(self.from, self.to, self.progress)
    }
    
    fn update(&mut self, dt: f32) {
        self.progress = (self.progress + dt).min(1.0);
    }
}
```

## Testing Styles

```rust
#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_style_patch() {
        let base = Style::default().fg(Color::Red);
        let overlay = Style::default().bg(Color::Blue);
        let result = base.patch(overlay);
        
        assert_eq!(result.fg, Some(Color::Red));
        assert_eq!(result.bg, Some(Color::Blue));
    }
    
    #[test]
    fn test_style_override() {
        let base = Style::default().fg(Color::Red);
        let overlay = Style::default().fg(Color::Blue);
        let result = base.patch(overlay);
        
        assert_eq!(result.fg, Some(Color::Blue));  // Overridden
    }
    
    #[test]
    fn test_modifier_combination() {
        let base = Style::default().add_modifier(Modifier::BOLD);
        let overlay = Style::default().add_modifier(Modifier::ITALIC);
        let result = base.patch(overlay);
        
        assert!(result.add_modifier.contains(Modifier::BOLD));
        assert!(result.add_modifier.contains(Modifier::ITALIC));
    }
}
```

## Best Practices

1. **Define base styles once**: Create a theme struct with all your base styles
2. **Use patch_style() for enhancements**: When you want to keep existing styling
3. **Use style() for replacements**: When you want to completely replace styling
4. **Compose from base**: Build complex styles by patching simple ones
5. **Test your themes**: Ensure styles compose as expected
6. **Consider accessibility**: High contrast options, colorblind-friendly palettes
7. **Document your theme**: Make it clear what each style is for

## Performance Considerations

- Style patching is very cheap (just struct field copying)
- Create theme structs once, reuse them
- Don't recreate styles on every frame
- Consider lazy style evaluation for complex themes

## Further Reading

- [Style API Documentation](https://docs.rs/ratatui/latest/ratatui/style/struct.Style.html)
- [Stylize Trait](https://docs.rs/ratatui/latest/ratatui/style/trait.Stylize.html)
- [Color Palettes](https://docs.rs/ratatui/latest/ratatui/style/palette/index.html)
- [Styling Text Guide](https://ratatui.rs/recipes/render/style-text/)
       }
    }
    
    fn build(self) -> Style {
        self.style
    }
}

// Usage
let style = StyleBuilder::new()
    .fg(Color::White)
    .bg(Color::Black)
    .when(is_important, |b| b.bold())
    .when(is_highlighted, |b| b.fg(Color::Yellow))
    .build();
```

## Color Palettes

Ratatui provides built-in color palettes for consistent theming:

### Material Design Colors

```rust
use ratatui::style::palette::material::*;

let theme = Style::default()
    .fg(BLUE.c500)
    .bg(BLUE.c50);

let error = Style::default()
    .fg(RED.c500)
    .add_modifier(Modifier::BOLD);

let success = Style::default()
    .fg(GREEN.c500);

// Available palettes:
// RED, PINK, PURPLE, DEEP_PURPLE, INDIGO, BLUE,
// LIGHT_BLUE, CYAN, TEAL, GREEN, LIGHT_GREEN,
// LIME, YELLOW, AMBER, ORANGE, DEEP_ORANGE,
// BROWN, GREY, BLUE_GREY
```

### Tailwind CSS Colors

```rust
use ratatui::style::palette::tailwind::*;

let theme = Style::default()
    .fg(BLUE.c500)
    .bg(SLATE.c900);

let button = Style::default()
    .fg(SKY.c50)
    .bg(SKY.c600)
    .add_modifier(Modifier::BOLD);

// Similar palette names as Material Design
```

## Serializable Themes with Serde

Enable the `serde` feature to save/load themes:

```rust
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
pub struct SerializableTheme {
    pub primary: (u8, u8, u8),
    pub background: (u8, u8, u8),
    pub text: (u8, u8, u8),
    pub error: (u8, u8, u8),
}

impl SerializableTheme {
    pub fn to_theme(&self) -> Theme {
        Theme {
            base: Style::default()
                .bg(Color::Rgb(self.background.0, self.background.1, self.background.2))
                .fg(Color::Rgb(self.text.0, self.text.1, self.text.2)),
            primary: Color::Rgb(self.primary.0, self.primary.1, self.primary.2),
            error: Color::Rgb(self.error.0, self.error.1, self.error.2),
            // ... other fields
        }
    }
    
    pub fn save(&self, path: &str) -> Result<(), Box<dyn std::error::Error>> {
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(path, json)?;
        Ok(())
    }
    
    pub fn load(path: &str) -> Result<Self, Box<dyn std::error::Error>> {
        let json = std::fs::read_to_string(path)?;
        Ok(serde_json::from_str(&json)?)
    }
}
```

## Common Pitfalls

### ❌ Using style() When You Mean patch_style()

```rust
// Wrong - loses the red color
let span = "error".red();
let span = span.style(Style::default().bold());  // Now ONLY bold!
```

```rust
// Right - keeps red, adds bold
let span = "error".red();
let span = span.patch_style(Style::default().bold());  // Red AND bold
```

### ❌ Not Composing Base Styles

```rust
// Wrong - repeating background everywhere
let header = Style::default().fg(Color::White).bg(Color::Blue);
let content = Style::default().fg(Color::White).bg(Color::Blue);
let footer = Style::default().fg(Color::Gray).bg(Color::Blue);
```

```rust
// Right - compose from base
let base = Style::default().bg(Color::Blue);
let header = base.fg(Color::White);
let content = base.fg(Color::White);
let footer = base.fg(Color::Gray);
```

### ❌ Forgetting Modifier Behavior

```rust
// This removes bold!
let style = Style::default()
    .add_modifier(Modifier::BOLD)
    .remove_modifier(Modifier::BOLD);  // Gone!
```

```rust
// Use sub_modifier for "never apply"
let style = Style::default()
    .sub_modifier(Modifier::BOLD);  // Won't be bold even if patched
```

## Advanced Techniques

### Style Inheritance System

```rust
struct StyleInheritance {
    global: Style,
    parent: Option<Style>,
    local: Style,
}

impl StyleInheritance {
    fn resolve(&self) -> Style {
        let mut result = self.global;
        
        if let Some(parent) = self.parent {
            result = result.patch(parent);
        }
        
        result.patch(self.local)
    }
}

// Usage in component tree
struct Component {
    style: Style,
    children: Vec<Component>,
}

impl Component {
    fn render(&self, frame: &mut Frame, area: Rect, parent_style: Style) {
        let final_style = parent_style.patch(self.style);
        
        // Render with final_style
        
        // Pass style to children
        for child in &self.children {
            child.render(frame, child_area, final_style);
        }
    }
}
```

### Dynamic Theme Switching

```rust
struct App {
    theme: Theme,
    theme_id: usize,
}

impl App {
    fn cycle_theme(&mut self) {
        self.theme_id = (self.theme_id + 1) % 3;
        self.theme = match self.theme_id {
            0 => Theme::light(),
            1 => Theme::dark(),
            _ => Theme::high_contrast(),
        };
    }
    
    fn render(&self, frame: &mut Frame, area: Rect) {
        let paragraph = Paragraph::new("Hello")
            .style(self.theme.normal);
        frame.render_widget(paragraph, area);
    }
}
```

### Style Interpolation

```rust
fn interpolate_color(from: Color, to: Color, t: f32) -> Color {
    match (from, to) {
        (Color::Rgb(r1, g1, b1), Color::Rgb(r2, g2, b2)) => {
            let r = (r1 as f32 + (r2 as f32 - r1 as f32) * t) as u8;
            let g = (g1 as f32 + (g2 as f32 - g1 as f32) * t) as u8;
            let b = (b1 as f32 + (b2 as f32 - b1 as f32) * t) as u8;
            Color::Rgb(r, g, b)
        }
        _ => from,
    }
}

fn interpolate_style(from: Style, to: Style, t: f32) -> Style {
    let fg = match (from.fg, to.fg) {
        (Some(from_fg), Some(to_fg)) => Some(interpolate_color(from_fg, to_fg, t)),
        _ => from.fg.or    span.patch_style(Style::default().bold())  // Adds bold to existing style
}
```

## The patch() Method

The `patch()` method on `Style` merges two styles together:

```rust
use ratatui::style::{Style, Color, Modifier};

let base = Style::default()
    .fg(Color::Blue)
    .add_modifier(Modifier::ITALIC);

let patch = Style::default()
    .bg(Color::Yellow)
    .add_modifier(Modifier::BOLD);

let result = base.patch(patch);

// Result has:
// - fg: Blue (from base)
// - bg: Yellow (from patch)
// - modifiers: ITALIC | BOLD (combined)
```

### Patch Rules

When patching styles, the patch takes precedence for colors, but modifiers are **combined**:

```rust
let base = Style::default().fg(Color::Red).italic();
let patch = Style::default().fg(Color::Blue).bold();

let result = base.patch(patch);
// fg: Blue (patch overwrites base)
// modifiers: ITALIC | BOLD (combined)
```

## Style Hierarchy and Inheritance

Styles flow down through the rendering hierarchy:

```
Widget Style
    ↓ (patches)
Block Style
    ↓ (patches)
Text Style
    ↓ (patches)
Line Style
    ↓ (patches)
Span Style
```

### Example: Complete Flow

```rust
use ratatui::{
    widgets::{Block, Paragraph},
    text::{Line, Span, Text},
    style::{Style, Color, Modifier},
};

// Each level adds to the previous level
let paragraph = Paragraph::new(
    Text::from(vec![
        Line::from(vec![
            Span::raw("Normal"),        // Gets widget + block + text styles
            Span::styled(
                "Bold",
                Style::default().bold()  // Adds bold to inherited styles
            ),
        ])
        .style(Style::default().italic()),  // Adds italic to all spans
    ])
    .style(Style::default().fg(Color::Yellow))  // Sets base color
)
.block(Block::bordered()
    .style(Style::default().bg(Color::Blue)))  // Sets background
.style(Style::default().fg(Color::White));     // Widget level (overrides Text fg)

// Final rendering:
// - "Normal": white fg, blue bg, italic (from line)
// - "Bold": white fg, blue bg, italic (from line) + bold (from span)
```

## Practical Patterns

### Theme Systems

Build a consistent theme using base styles:

```rust
pub struct Theme {
    pub base: Style,
    pub primary: Style,
    pub secondary: Style,
    pub success: Style,
    pub warning: Style,
    pub error: Style,
    pub muted: Style,
}

impl Theme {
    pub fn new() -> Self {
        let base = Style::default()
            .fg(Color::White)
            .bg(Color::Black);
        
        Self {
            base,
            primary: base.patch(Style::default().fg(Color::Blue)),
            secondary: base.patch(Style::default().fg(Color::Cyan)),
            success: base.patch(Style::default().fg(Color::Green)),
            warning: base.patch(Style::default().fg(Color::Yellow)),
            error: base.patch(Style::default().fg(Color::Red).bold()),
            muted: base.patch(Style::default().fg(Color::DarkGray)),
        }
    }
}

// Usage
fn render(frame: &mut Frame, area: Rect, theme: &Theme) {
    let text = Text::from(vec![
        Line::from(vec![
            "Status: ".into(),
            "OK".patch_style(theme.success),
        ]),
        Line::from(vec![
            "Error: ".into(),
            "Failed".patch_style(theme.error),
        ]),
    ])
    .style(theme.base);
    
    frame.render_widget(Paragraph::new(text), area);
}
```

### State-Based Styling

Enhance styles based on widget state:

```rust
struct Button {
    label: String,
    is_focused: bool,
    is_pressed: bool,
}

impl Button {
    fn style(&self) -> Style {
        let base = Style::default()
            .fg(Color::White)
            .bg(Color::Blue);
        
        let style = if self.is_focused {
            base.patch(Style::default().bold())
        } else {
            base
        };
        
        if self.is_pressed {
            style.patch(Style::default()
                .bg(Color::DarkBlue)
                .add_modifier(Modifier::REVERSED))
        } else {
            style
        }
    }
    
    fn render(&self, area: Rect, buf: &mut Buffer) {
        let block = Block::bordered()
            .style(self.style());
        
        let inner = block.inner(area);
        block.render(area, buf);
        
        let label = Line::from(self.label.as_str())
            .centered()
            .patch_style(self.style());  // Inherits button style
        
        label.render(inner, buf);
    }
}
```

### Modifier Combination

Modifiers are combined using bitwise OR:

```rust
// Build up