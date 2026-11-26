---
tags: [diagram]
---
# Flow

A small flowchart:

```mermaid
flowchart LR
    A[Draft] --> B[Review]
    B --> C{Approved?}
    C -->|yes| D[Publish]
    C -->|no| A
```
