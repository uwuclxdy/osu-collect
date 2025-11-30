# Guide to Blocks (boxes) in Ratatui

## Table of Contents
1. [Introduction](#introduction)
2. [Basic Usage](#basic-usage)
3. [Creating Blocks](#creating-blocks)
4. [Borders](#borders)
5. [Border Types](#border-types)
6. [Border Sets and Custom Symbols](#border-sets-and-custom-symbols)
7. [Titles](#titles)
8. [Padding](#padding)
9. [Styling](#styling)
10. [Inner Area Calculation](#inner-area-calculation)
11. [Advanced Usage](#advanced-usage)
12. [Best Practices](#best-practices)
13. [Common Patterns](#common-patterns)

---

## Introduction

The Block widget is a foundational building block in Ratatui that displays a box border around other widgets and can have borders, titles, and styling elements to enhance the structure of terminal interfaces. Block is a basic widget that draws a block with optional borders, titles and styles.

### Key Features
- Configurable borders (all sides, selective sides, or none)
- Multiple border styles and types
- Support for multiple titles with flexible positioning
- Padding control
- Style customization for borders, titles, and content
- Calculation of inner rendering area

---

## Basic Usage

### Simple Block with Borders

The simplest Block creates a container with all borders:

```rust
use ratatui::widgets::{Block, Borders};

let block = Block::default()
    .borders(Borders::ALL);

frame.render_widget(block, area);
```

### Block with Bordered Shorthand

```rust
use ratatui::widgets::Block;

// This is equivalent to .borders(Borders::ALL)
let block = Block::bordered();

frame.render_widget(block, area);
```

---

## Creating Blocks

### Basic Construction

Block::new creates a new Block with no border or padding:

```rust
use ratatui::widgets::Block;

// Empty block with no borders
let block = Block::new();

// Block with all borders
let block = Block::bordered();

// Block using default
let block = Block::default();
```

---

## Borders

### Border Configuration

Borders can be configured with Block::borders. You can specify which borders to display using bitflags:

```rust
use ratatui::widgets::{Block, Borders};

// All borders
let block = Block::new().borders(Borders::ALL);

// Specific borders
let block = Block::new()
    .borders(Borders::LEFT | Borders::RIGHT);

// Top and bottom only
let block = Block::new()
    .borders(Borders::TOP | Borders::BOTTOM);

// Single border
let block = Block::new().borders(Borders::LEFT);

// No borders
let block = Block::new().borders(Borders::NONE);
```

### Available Border Flags

- `Borders::NONE` - No borders
- `Borders::TOP` - Top border only
- `Borders::BOTTOM` - Bottom border only
- `Borders::LEFT` - Left border only
- `Borders::RIGHT` - Right border only
- `Borders::ALL` - All borders

These can be combined using the `|` (bitwise OR) operator.

---

## Border Types

Block supports different border types through the border_type method, which sets the symbols used to display borders.

### Available Border Types

```rust
use ratatui::widgets::{Block, BorderType};

// Plain (default) - Single line borders
let block = Block::bordered()
    .border_type(BorderType::Plain);
// ┌─────┐
// │     │
// └─────┘

// Rounded - Curved corners
let block = Block::bordered()
    .border_type(BorderType::Rounded);
// ╭─────╮
// │     │
// ╰─────╯

// Double - Double line borders
let block = Block::bordered()
    .border_type(BorderType::Double);
// ╔═════╗
// ║     ║
// ╚═════╝

// Thick - Thick line borders
let block = Block::bordered()
    .border_type(BorderType::Thick);
// ┏━━━━━┓
// ┃     ┃
// ┗━━━━━┛
```

### Additional Border Types

QuadrantInside and QuadrantOutside border types use unicode quadrant characters that look like half block pixels:

```rust
// QuadrantOutside
let block = Block::bordered()
    .border_type(BorderType::QuadrantOutside);
// ▛▀▀▀▀▜
// ▌    ▐
// ▙▄▄▄▄▟

// QuadrantInside
let block = Block::bordered()
    .border_type(BorderType::QuadrantInside);
// ▗▄▄▄▄▖
// ▐    ▌
// ▝▀▀▀▀▘
```

---

## Border Sets and Custom Symbols

Applications can set custom borders on a Block by calling border_set, which allows specifying the exact symbols used for each part of the border.

### Custom Border Symbols

```rust
use ratatui::symbols::border;
use ratatui::widgets::{Block, Borders};

let block = Block::default()
    .borders(Borders::ALL)
    .border_set(border::Set {
        top_left: "1",
        top_right: "2",
        bottom_left: "3",
        bottom_right: "4",
        vertical_left: "L",
        vertical_right: "R",
        horizontal_top: "T",
        horizontal_bottom: "B",
    });
// 1TTTTTTT2
// L       R
// 3BBBBBBB4
```

### Predefined Border Sets

```rust
use ratatui::{symbols, widgets::Block};

// Using predefined symbol sets
let block = Block::bordered()
    .border_set(symbols::border::PLAIN);

let block = Block::bordered()
    .border_set(symbols::border::ROUNDED);

let block = Block::bordered()
    .border_set(symbols::border::DOUBLE);

let block = Block::bordered()
    .border_set(symbols::border::THICK);

// Special proportional tall borders
let block = Block::bordered()
    .border_set(symbols::border::PROPORTIONAL_TALL);
```

---

## Titles

A Block can have multiple titles using Block::title, and each title is rendered with a single space separating titles in the same position or alignment.

### Basic Title

```rust
use ratatui::widgets::Block;

let block = Block::bordered()
    .title("My Title");

frame.render_widget(block, area);
```

### Multiple Titles with Positioning

```rust
use ratatui::{
    text::Line,
    widgets::Block,
};

// Multiple titles with different alignments
let block = Block::bordered()
    .title(Line::from("Left Title").left_aligned())
    .title(Line::from("Center Title").centered())
    .title(Line::from("Right Title").right_aligned());

// Renders:
// ┌Left Title──Center Title───────Right Title┐
```

### Top and Bottom Titles

```rust
use ratatui::{
    text::Line,
    widgets::Block,
};

let block = Block::bordered()
    .title("Top Title")
    .title_bottom(Line::from("Bottom Left").left_aligned())
    .title_bottom(Line::from("Bottom Center").centered())
    .title_bottom(Line::from("Bottom Right").right_aligned());
```

### Title Positioning with Title Struct

```rust
use ratatui::widgets::{
    Block,
    block::{Title, Position},
};

let block = Block::new()
    .title(Title::from("Title 1"))
    .title(Title::from("Title 2").position(Position::Bottom));
```

### Title Styling

The title_style method applies style to all titles, which is applied after Block::style or Block::border_style:

```rust
use ratatui::{
    style::{Color, Style},
    widgets::Block,
};

let block = Block::bordered()
    .title("Styled Title")
    .title_style(Style::default()
        .fg(Color::Yellow)
        .bold());
```

### Title Alignment

Block::title_alignment sets the default alignment for all block titles:

```rust
use ratatui::{
    layout::Alignment,
    widgets::Block,
};

let block = Block::bordered()
    .title_alignment(Alignment::Center)
    .title("Centered Title")
    .title("Another Centered Title");
```

### Important Title Behavior

When both centered and non-centered titles are rendered, the centered space is calculated based on the full width of the block rather than leftover width. Titles are not rendered in the corners of the block unless there is no border on that edge.

---

## Padding

Padding defines the internal spacing inside a Block.

### Creating Padding

```rust
use ratatui::widgets::{Block, Padding};

// Using the constructor with (left, right, top, bottom)
let block = Block::bordered()
    .padding(Padding::new(5, 10, 1, 2));

// Uniform padding on all sides
let block = Block::bordered()
    .padding(Padding::uniform(1));

// Horizontal padding only (left and right)
let block = Block::bordered()
    .padding(Padding::horizontal(2));

// Vertical padding only (top and bottom)
let block = Block::bordered()
    .padding(Padding::vertical(2));

// Symmetric padding (x for horizontal, y for vertical)
let block = Block::bordered()
    .padding(Padding::symmetric(5, 6));

// Individual side padding
let block = Block::bordered()
    .padding(Padding::left(3));
    
let block = Block::bordered()
    .padding(Padding::right(3));
    
let block = Block::bordered()
    .padding(Padding::top(1));
    
let block = Block::bordered()
    .padding(Padding::bottom(1));

// Proportional padding (visually proportional to terminal)
let block = Block::bordered()
    .padding(Padding::proportional(4));
```

### Padding Examples

```rust
use ratatui::widgets::{Block, Padding};

// Left padding of 2, affecting content placement
let block = Block::bordered()
    .padding(Padding::horizontal(2));
// ┌───────────┐
// │  content  │
// └───────────┘
```

---

## Styling

Styles are applied first to the entire block, then to the borders, and finally to the titles.

### Block Style

```rust
use ratatui::{
    style::{Color, Style},
    widgets::Block,
};

// Base style for the block
let block = Block::bordered()
    .style(Style::default()
        .bg(Color::Black)
        .fg(Color::White));
```

### Border Style

border_style defines the style of borders and is applied after Block::style:

```rust
use ratatui::{
    style::{Color, Style},
    widgets::{Block, BorderType},
};

let block = Block::bordered()
    .border_type(BorderType::Rounded)
    .border_style(Style::default()
        .fg(Color::Magenta));
```

### Style Layering

```rust
use ratatui::{
    style::{Color, Style, Stylize},
    widgets::{Block, Paragraph},
};

// Styles are layered: Block -> Border -> Title -> Inner Widget
let block = Block::new()
    .style(Style::new().red().bold().italic())
    .border_style(Style::new().not_italic())  // red and bold
    .title_style(Style::new().not_bold())      // red and italic
    .title("Title");

// Inner widget can have its own style
let paragraph = Paragraph::new("Content")
    .block(block)
    .style(Style::new().white().not_bold());  // white and italic
```

### Using Stylize Trait

The `Stylize` trait provides convenient shorthand methods:

```rust
use ratatui::{
    style::Stylize,
    widgets::Block,
};

let block = Block::bordered()
    .red()
    .on_black()
    .bold()
    .italic();
```

---

## Inner Area Calculation

The inner method computes the inner area of a block based on its border visibility rules.

### Using the Inner Method

```rust
use ratatui::widgets::Block;

let outer_block = Block::bordered()
    .title("Outer Block");

// Calculate the inner area (excluding borders and padding)
let inner_area = outer_block.inner(area);

// Render the outer block
frame.render_widget(outer_block, area);

// Render content in the inner area
frame.render_widget(content, inner_area);
```

### Nested Blocks Example

```rust
use ratatui::widgets::Block;

let outer_block = Block::bordered()
    .title("Outer");

let inner_block = Block::bordered()
    .title("Inner");

// Calculate inner area
let inner = outer_block.inner(area);

// Render both blocks
frame.render_widget(outer_block, area);
frame.render_widget(inner_block, inner);
```

### Inner Area with Padding

Previously, when computing the inner rendering area of a block, all titles were assumed to be at the top, but this was fixed to make inner aware of title positions:

```rust
use ratatui::widgets::{Block, Padding};

let block = Block::bordered()
    .padding(Padding::uniform(2));

// The inner area accounts for borders and padding
let inner = block.inner(area);
```

---

## Advanced Usage

### Block as Container for Other Widgets

A Block can be passed as a parameter to another widget so that the block surrounds it:

```rust
use ratatui::widgets::{Block, Borders, List};

let surrounding_block = Block::default()
    .borders(Borders::ALL)
    .title("List Container");

let items = ["Item 1", "Item 2", "Item 3"];
let list = List::new(items)
    .block(surrounding_block);

frame.render_widget(list, area);
```

### Combining Border Configurations

```rust
use ratatui::widgets::{Block, BorderType, Borders};

// Combining different border configurations
let block = Block::new()
    .borders(Borders::TOP | Borders::BOTTOM)
    .border_type(BorderType::Double);
```

### Dynamic Border Selection

```rust
use ratatui::widgets::{Block, Borders};

fn create_block(show_borders: bool) -> Block<'static> {
    let borders = if show_borders {
        Borders::ALL
    } else {
        Borders::NONE
    };
    
    Block::bordered().borders(borders)
}
```

---

## Best Practices

### 1. Use Appropriate Border Types

Choose border types that match your UI aesthetic:
- **Plain**: Default, clean look for most applications
- **Rounded**: Modern, softer appearance
- **Double**: Emphasize important sections
- **Thick**: Draw attention to critical areas

### 2. Title Positioning

Each title will be rendered with a single space separating titles in the same position or alignment:

```rust
// Good: Clear separation of concerns
let block = Block::bordered()
    .title("Main Title")
    .title_bottom("Status Info");

// Good: Multiple related items
let block = Block::bordered()
    .title(Line::from("File").left_aligned())
    .title(Line::from("Modified").centered())
    .title(Line::from("100%").right_aligned());
```

### 3. Consistent Styling

Apply consistent styles throughout your application:

```rust
use ratatui::style::{Color, Style};

const HEADER_STYLE: Style = Style::new()
    .fg(Color::Yellow);

let block = Block::bordered()
    .title("Header")
    .title_style(HEADER_STYLE);
```

### 4. Inner Area Calculation

Always calculate inner area when rendering content within blocks:

```rust
let block = Block::bordered().padding(Padding::uniform(1));
let inner = block.inner(area);

frame.render_widget(block, area);
frame.render_widget(content, inner);
```

### 5. Padding for Readability

Use padding to improve content readability:

```rust
use ratatui::widgets::{Block, Padding, Paragraph};

let block = Block::bordered()
    .padding(Padding::horizontal(2))  // Breathing room
    .title("Content");

let paragraph = Paragraph::new("Your text here")
    .block(block);
```

---

## Common Patterns

### Pattern 1: Section Headers

```rust
use ratatui::{
    style::{Color, Style, Stylize},
    widgets::Block,
};

fn section_block(title: &str) -> Block {
    Block::bordered()
        .title(title)
        .title_style(Style::default().fg(Color::Yellow).bold())
        .border_style(Style::default().fg(Color::Gray))
}
```

### Pattern 2: Status Display

```rust
use ratatui::{
    text::Line,
    widgets::Block,
};

fn status_block(title: &str, status: &str) -> Block {
    Block::bordered()
        .title(Line::from(title).left_aligned())
        .title(Line::from(status).right_aligned())
}
```

### Pattern 3: Focused Widget

```rust
use ratatui::{
    style::{Color, Style},
    widgets::{Block, BorderType},
};

fn focused_block(is_focused: bool) -> Block<'static> {
    let border_style = if is_focused {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default().fg(Color::Gray)
    };
    
    let border_type = if is_focused {
        BorderType::Thick
    } else {
        BorderType::Plain
    };
    
    Block::bordered()
        .border_type(border_type)
        .border_style(border_style)
}
```

### Pattern 4: Info Box

```rust
use ratatui::widgets::{Block, Padding};

fn info_box(title: &str) -> Block {
    Block::bordered()
        .title(title)
        .padding(Padding::uniform(1))
}
```

### Pattern 5: Popup/Dialog

```rust
use ratatui::{
    style::{Color, Style},
    widgets::{Block, BorderType},
};

fn dialog_block(title: &str) -> Block {
    Block::bordered()
        .title(title)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(Color::White))
        .style(Style::default().bg(Color::Black))
}
```

### Pattern 6: Minimal Block

```rust
use ratatui::widgets::{Block, Borders};

fn minimal_block() -> Block<'static> {
    Block::new()
        .borders(Borders::NONE)
        .style(Style::default().bg(Color::DarkGray))
}
```

### Pattern 7: Collapsing Borders

For creating layouts where borders touch and merge:

```rust
use ratatui::{
    symbols::border,
    widgets::{Block, Borders},
};

// Create custom border sets to merge borders between adjacent blocks
let top_left = Block::bordered();

let top_right = Block::default()
    .borders(Borders::TOP | Borders::RIGHT | Borders::BOTTOM)
    .border_set(border::Set {
        top_left: border::NORMAL.horizontal_top,
        ..border::NORMAL
    });

let bottom_right = Block::default()
    .borders(Borders::RIGHT | Borders::BOTTOM)
    .border_set(border::Set {
        top_left: border::NORMAL.vertical_left,
        top_right: border::NORMAL.horizontal_top,
        ..border::NORMAL
    });
```

---

## Integration Examples

### With Paragraph

```rust
use ratatui::widgets::{Block, Paragraph};

let block = Block::bordered().title("Text Content");
let paragraph = Paragraph::new("Your text here")
    .block(block);

frame.render_widget(paragraph, area);
```

### With List

```rust
use ratatui::widgets::{Block, List};

let block = Block::bordered().title("Items");
let items = ["Item 1", "Item 2", "Item 3"];
let list = List::new(items).block(block);

frame.render_widget(list, area);
```

### With Table

```rust
use ratatui::widgets::{Block, Table, Row};

let block = Block::bordered().title("Data Table");
let rows = vec![
    Row::new(vec!["Cell 1", "Cell 2"]),
];

let table = Table::new(rows, [Constraint::Percentage(50), Constraint::Percentage(50)])
    .block(block);

frame.render_widget(table, area);
```

---

## Tips and Tricks

### 1. No Border Block for Spacing

Use blocks without borders for layout spacing:

```rust
use ratatui::widgets::{Block, Borders, Padding};

let spacer = Block::new()
    .borders(Borders::NONE)
    .padding(Padding::uniform(1));
```

### 2. Conditional Borders

```rust
fn create_conditional_block(condition: bool) -> Block<'static> {
    let mut block = Block::new();
    
    if condition {
        block = block.borders(Borders::ALL);
    }
    
    block
}
```

### 3. Builder Pattern

Blocks use the builder pattern extensively:

```rust
let block = Block::bordered()
    .title("Title")
    .border_type(BorderType::Rounded)
    .border_style(Style::default().fg(Color::Blue))
    .padding(Padding::uniform(1))
    .style(Style::default().bg(Color::Black));
```

### 4. Reusable Block Styles

Create functions for reusable block styles:

```rust
use ratatui::{
    style::{Color, Style, Stylize},
    widgets::{Block, BorderType},
};

fn app_block(title: &str) -> Block {
    Block::bordered()
        .title(title)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(Color::Cyan))
}

fn error_block(title: &str) -> Block {
    Block::bordered()
        .title(title)
        .border_style(Style::default().fg(Color::Red).bold())
}

fn success_block(title: &str) -> Block {
    Block::bordered()
        .title(title)
        .border_style(Style::default().fg(Color::Green))
}
```

---

## Performance Considerations

1. **Avoid Recreating Blocks Every Frame**: If your block configuration is static, consider storing it or creating it once.

2. **Use References**: When possible, use references to avoid unnecessary cloning.

3. **Conditional Rendering**: Only render borders when they're visible to the user.

---

## Troubleshooting

### Issue: Titles Overlapping

If the block is too small and multiple titles overlap, the border may get cut off at a corner.

**Solution**: Ensure adequate space or reduce the number of titles.

### Issue: Centered Title Not Centered

When both centered and non-centered titles are rendered, the centered space is calculated based on the full width of the block, not the leftover width.

**Solution**: This is expected behavior. Plan title layouts accordingly.

### Issue: Title in Corner

Titles are not rendered in the corners unless there is no border on that edge.

**Solution**: Adjust border configuration or title positioning.

### Issue: Inner Area Too Small

**Solution**: Check padding and border settings. Reduce padding if necessary.

```rust
let block = Block::bordered()
    .padding(Padding::horizontal(1));  // Reduced padding

let inner = block.inner(area);
```

---

## Migration Notes

### From Earlier Versions

BorderType::line_symbols has been renamed to border_symbols and now returns symbols::border::Set instead of symbols::line::Set.

```rust
// Old (pre-v0.24.0)
// let line_set: symbols::line::Set = BorderType::line_symbols(BorderType::Plain);

// New (v0.24.0+)
use ratatui::widgets::BorderType;

let border_set: symbols::border::Set = BorderType::border_symbols(BorderType::Plain);
```

---

## Conclusion

The Block widget is a fundamental component in Ratatui that provides structure and visual organization to terminal UIs. By mastering blocks, you can:

- Create well-structured layouts
- Add visual hierarchy to your interface
- Improve user experience with clear boundaries
- Build professional-looking terminal applications

Key takeaways:
- Blocks are containers with configurable borders, titles, padding, and styles
- Use `Block::bordered()` for quick block creation with all borders
- Leverage the `inner()` method to calculate content areas
- Apply consistent styling for a cohesive user interface
- Combine blocks with other widgets for rich terminal UIs

For more information, visit:
- [Ratatui Documentation](https://ratatui.rs)
- [Ratatui API Docs](https://docs.rs/ratatui/)
- [Ratatui GitHub](https://github.com/ratatui/ratatui)
