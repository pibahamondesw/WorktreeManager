---
name: wtm-html-preview
description: Use a self-contained HTML preview in WorktreeManager chat when a static chart, rich table, or mockup makes an answer easier to understand.
---

# Respond with an HTML preview

Use this format directly when a visual helps answer the user, even if they did not ask for HTML. Use ordinary Markdown for prose, lists, and simple tables. Keep the main takeaway in the surrounding message.

Send a fenced `html preview` block containing an HTML fragment. WorktreeManager supplies the document and renders the preview once the closing fence arrives. Use a plain `html` fence when the user needs to read or copy the source.

````markdown
```html preview
<figure aria-label="12 of 16 tasks complete">
  <figcaption>12 of 16 tasks complete</figcaption>
  <div style="max-width: 24rem; height: 1rem; background: var(--color-bg-tertiary)">
    <div style="width: 75%; height: 100%; background: var(--color-accent)"></div>
  </div>
</figure>
```
````

## Preview limits

- Keep the page self-contained. The sandbox disables scripts and blocks network resources. Use HTML and inline CSS in a `<style>` block or `style` attribute. Omit JavaScript, external stylesheets, and dependencies that require a network request.
- Embed images and custom fonts with `data:` URLs. Inline SVG works for static charts and diagrams. Use system fonts when possible.
- Use the injected `--color-*` theme tokens, such as `--color-bg-secondary`, `--color-text-primary`, `--color-border`, and `--color-accent`. Keep labels legible in light and dark themes.
