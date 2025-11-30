# Ratatui Canvas Widget - Complete Guide

## Overview

The Canvas widget is often overlooked when developers need custom visualizations. Instead of manually drawing with buffer manipulation, Canvas provides a coordinate-based system for drawing shapes (lines, circles, rectangles, etc.) with automatic mapping to terminal cells using Braille patterns, half blocks, or other markers.

## Key Benefits

- **Coordinate system**: Work in floating-point coordinates, Canvas handles terminal mapping
- **Multiple marker types**: Braille (2x4 dots), Half blocks, Dots, Blocks
- **Layering support**: Draw shapes in specific order
- **Built-in shapes**: Lines, Circles, Rectangles, Maps, and custom shapes
- **Text overlay**: Print text on top of shapes

## Basic Usage

```rust
use ratatui::{
    style::Color,
    widgets::canvas::{Canvas, Circle, Line, Rectangle},
    Frame,
};

fn render(frame: &mut Frame, area: Rect) {
    let canvas = Canvas::default()
        .block(Block::bordered().title("Canvas"))
        .x_bounds([-10.0, 10.0])      // Coordinate system X
        .y_bounds([-10.0, 10.0])      // Coordinate system Y  
        .paint(|ctx| {
            // Draw a circle
            ctx.draw(&Circle {
                x: 0.0,
                y: 0.0,
                radius: 5.0,
                color: Color::Cyan,
            });
            
            // Draw a line
            ctx.draw(&Line {
                x1: -5.0,
                y1: -5.0,
                x2: 5.0,
                y2: 5.0,
                color: Color::White,
            });
        });
    
    frame.render_widget(canvas, area);
}
```

## Marker Types

### Braille (Default) - Highest Resolution
```rust
use ratatui::symbols::Marker;

let canvas = Canvas::default()
    .marker(Marker::Braille)  // 2x4 dots per cell
    .x_bounds([0.0, 100.0])
    .y_bounds([0.0, 100.0]);
```

### HalfBlock - Supports Foreground AND Background
```rust
let canvas = Canvas::default()
    .marker(Marker::HalfBlock)  // Upper/lower half blocks
    // Can set both foreground and background colors!
```

### Dot and Block
```rust
// Simple dot marker
let canvas = Canvas::default().marker(Marker::Dot);

// Block character marker
let canvas = Canvas::default().marker(Marker::Block);
```

## Built-in Shapes

### Circle
```rust
use ratatui::widgets::canvas::Circle;

ctx.draw(&Circle {
    x: 0.0,        // Center X
    y: 0.0,        // Center Y
    radius: 10.0,
    color: Color::Red,
});
```

### Line
```rust
use ratatui::widgets::canvas::Line;

ctx.draw(&Line {
    x1: 0.0,
    y1: 0.0,
    x2: 10.0,
    y2: 10.0,
    color: Color::Yellow,
});
```

### Rectangle
```rust
use ratatui::widgets::canvas::Rectangle;

ctx.draw(&Rectangle {
    x: 0.0,         // Top-left corner X
    y: 0.0,         // Top-left corner Y
    width: 10.0,
    height: 5.0,
    color: Color::Green,
});
```

### Map - World Map
```rust
use ratatui::widgets::canvas::{Map, MapResolution};

ctx.draw(&Map {
    resolution: MapResolution::High,
    color: Color::White,
});

// Available resolutions:
// MapResolution::Low
// MapResolution::High
```

### Points
```rust
use ratatui::widgets::canvas::Points;

let data = vec![
    (1.0, 2.0),
    (3.0, 4.0),
    (5.0, 6.0),
];

ctx.draw(&Points {
    coords: &data,
    color: Color::Magenta,
});
```

## Layering

Use `ctx.layer()` to control drawing order:

```rust
canvas.paint(|ctx| {
    // Background layer
    ctx.draw(&Rectangle {
        x: 0.0,
        y: 0.0,
        width: 100.0,
        height: 100.0,
        color: Color::Blue,
    });
    
    // Start new layer
    ctx.layer();
    
    // Foreground layer (drawn on top)
    ctx.draw(&Circle {
        x: 50.0,
        y: 50.0,
        radius: 20.0,
        color: Color::Red,
    });
});
```

## Text Overlay

```rust
canvas.paint(|ctx| {
    // Draw shapes
    ctx.draw(&Circle { x: 0.0, y: 0.0, radius: 5.0, color: Color::Cyan });
    
    // Add text (always on top, not affected by layers)
    ctx.print(0.0, 0.0, "Center", Color::White);
});
```

## Custom Shapes

Implement the `Shape` trait:

```rust
use ratatui::widgets::canvas::{Shape, Painter};

struct Triangle {
    x1: f64,
    y1: f64,
    x2: f64,
    y2: f64,
    x3: f64,
    y3: f64,
    color: Color,
}

impl Shape for Triangle {
    fn draw(&self, painter: &mut Painter) {
        // Draw three lines
        painter.paint_line(self.x1, self.y1, self.x2, self.y2, self.color);
        painter.paint_line(self.x2, self.y2, self.x3, self.y3, self.color);
        painter.paint_line(self.x3, self.y3, self.x1, self.y1, self.color);
    }
}

// Usage
ctx.draw(&Triangle {
    x1: 0.0, y1: 0.0,
    x2: 10.0, y2: 0.0,
    x3: 5.0, y3: 10.0,
    color: Color::Yellow,
});
```

## Real-World Examples

### Animated Particle System
```rust
struct Particle {
    x: f64,
    y: f64,
    vx: f64,
    vy: f64,
    color: Color,
}

struct ParticleSystem {
    particles: Vec<Particle>,
    bounds: (f64, f64),
}

impl ParticleSystem {
    fn update(&mut self, dt: f64) {
        for particle in &mut self.particles {
            particle.x += particle.vx * dt;
            particle.y += particle.vy * dt;
            
            // Bounce off walls
            if particle.x < 0.0 || particle.x > self.bounds.0 {
                particle.vx *= -1.0;
            }
            if particle.y < 0.0 || particle.y > self.bounds.1 {
                particle.vy *= -1.0;
            }
        }
    }
    
    fn render(&self, frame: &mut Frame, area: Rect) {
        let canvas = Canvas::default()
            .marker(Marker::Braille)
            .x_bounds([0.0, self.bounds.0])
            .y_bounds([0.0, self.bounds.1])
            .paint(|ctx| {
                for particle in &self.particles {
                    ctx.draw(&Circle {
                        x: particle.x,
                        y: particle.y,
                        radius: 1.0,
                        color: particle.color,
                    });
                }
            });
        
        frame.render_widget(canvas, area);
    }
}
```

### Function Plotter
```rust
fn plot_function<F>(f: F, x_range: (f64, f64), y_range: (f64, f64)) -> Canvas<'static, impl Fn(&mut Context) + 'static>
where
    F: Fn(f64) -> f64 + 'static,
{
    Canvas::default()
        .marker(Marker::Braille)
        .x_bounds([x_range.0, x_range.1])
        .y_bounds([y_range.0, y_range.1])
        .paint(move |ctx| {
            let points: Vec<(f64, f64)> = (0..1000)
                .map(|i| {
                    let x = x_range.0 + (x_range.1 - x_range.0) * (i as f64 / 1000.0);
                    let y = f(x);
                    (x, y)
                })
                .collect();
            
            ctx.draw(&Points {
                coords: &points,
                color: Color::Cyan,
            });
            
            // Draw axes
            ctx.draw(&Line { x1: x_range.0, y1: 0.0, x2: x_range.1, y2: 0.0, color: Color::White });
            ctx.draw(&Line { x1: 0.0, y1: y_range.0, x2: 0.0, y2: y_range.1, color: Color::White });
        })
}

// Usage: Plot sine wave
let canvas = plot_function(|x| x.sin(), (-10.0, 10.0), (-2.0, 2.0));
```

### Real-time Graph
```rust
struct RealtimeGraph {
    data: VecDeque<f64>,
    max_points: usize,
}

impl RealtimeGraph {
    fn add_point(&mut self, value: f64) {
        if self.data.len() >= self.max_points {
            self.data.pop_front();
        }
        self.data.push_back(value);
    }
    
    fn render(&self, frame: &mut Frame, area: Rect) {
        let points: Vec<(f64, f64)> = self.data
            .iter()
            .enumerate()
            .map(|(i, &y)| (i as f64, y))
            .collect();
        
        let canvas = Canvas::default()
            .marker(Marker::Braille)
            .x_bounds([0.0, self.max_points as f64])
            .y_bounds([0.0, 100.0])
            .paint(|ctx| {
                ctx.draw(&Points {
                    coords: &points,
                    color: Color::Green,
                });
            });
        
        frame.render_widget(canvas, area);
    }
}
```

## When to Use Canvas

**Use Canvas when:**
- Drawing at arbitrary coordinates
- Creating visualizations (graphs, plots, charts)
- Animations and particle effects
- Games (pong, asteroids, etc.)
- Custom shapes not provided by other widgets

**Don't use Canvas when:**
- Simple text rendering (use Paragraph)
- Standard charts (use Chart widget)
- Tables (use Table widget)
- Lists (use List widget)

## Further Reading

- [Canvas API](https://docs.rs/ratatui/latest/ratatui/widgets/canvas/struct.Canvas.html)
- [Canvas Example](https://ratatui.rs/examples/widgets/canvas/)
- [Shape Trait](https://docs.rs/ratatui/latest/ratatui/widgets/canvas/trait.Shape.html)
