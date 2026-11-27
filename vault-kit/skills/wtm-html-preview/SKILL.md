---
name: wtm-html-preview
description: Use a self-contained HTML preview in WorktreeManager chat when a chart, rich table, interactive view, or mockup makes an answer easier to understand.
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

## Scripts

Add JavaScript when interaction helps the user explore the answer: sorting or filtering a table, switching series, hovering for details, or a chart with many points.

- Write the HTML so the essential content reads without scripts, then let scripts enhance it. Windows previews run no scripts.
- Embed the data in the page, for example in a `<script type="application/json">` block. The page has no network: `fetch`, XHR, and WebSockets are blocked, and so are storage, forms, and links.
- Prefer inline `<script>` for simple interactions. Load a library only when it saves real work, from `https://cdn.jsdelivr.net` with a pinned version, such as `https://cdn.jsdelivr.net/npm/chart.js@4.4.1`. Libraries don't load offline, and they can't fetch stylesheets, fonts, or data from elsewhere.
- Read theme tokens from scripts with `getComputedStyle(document.documentElement).getPropertyValue("--color-accent")`, for example to color a canvas chart.

````markdown
```html preview
<table>
  <thead><tr><th>Step</th><th><button id="reverse" type="button">Minutes ↕</button></th></tr></thead>
  <tbody id="steps">
    <tr><td>Tests</td><td>31</td></tr>
    <tr><td>Build</td><td>12</td></tr>
    <tr><td>Deploy</td><td>4</td></tr>
  </tbody>
</table>
<script>
  const steps = document.getElementById("steps");
  document.getElementById("reverse").addEventListener("click", () => {
    steps.append(...[...steps.rows].reverse());
  });
</script>
```
````

## Preview limits

- Keep the page self-contained. Use HTML and inline CSS in a `<style>` block or `style` attribute; external stylesheets don't load.
- Embed images and custom fonts with `data:` URLs. Inline SVG works for charts and diagrams without scripts. Use system fonts when possible.
- Use the injected `--color-*` theme tokens, such as `--color-bg-secondary`, `--color-text-primary`, `--color-border`, and `--color-accent`. Keep labels legible in light and dark themes.
