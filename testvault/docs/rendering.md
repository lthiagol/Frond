---
tags: [markdown, rust]
---
# Rendering showcase

## Level two

### Level three

#### Level four

##### Level five

###### Level six

Lorem ipsum with **bold**, *italic*, ~~struck~~, ==marked== and `inline code`.

A footnote reference here.[^1]

[^1]: Lorem footnote text.

> Lorem blockquote with a second line.

> [!NOTE]
> Note callout body.

> [!TIP]
> Tip callout body.

> [!IMPORTANT]
> Important callout body.

> [!WARNING]
> Warning callout body.

> [!CAUTION]
> Caution callout body.

---

1. First ordered
2. Second ordered
   - nested bullet
   - nested bullet two

| Left | Center | Right |
|:-----|:------:|------:|
| a    | b      | c     |

```rust
fn main() {
    let answer: u32 = 42;
    println!("{answer}");
}
```

```python
def lorem(n: int) -> list[str]:
    return ["ipsum"] * n
```

```json
{"lorem": "ipsum", "dolor": [1, 2, 3]}
```

```bash
echo "lorem ipsum"
```

A fenced block is not a link, so this stays text:

```text
[[not-a-link]]
```

An external link: [example](https://example.com/lorem).
