// Mermaid syntax checker for markdown files (used by `make diagrams-check`).
//
// Usage: node check.mjs <file.md> [file.md ...]
//
// Extracts every ```mermaid fenced block from each markdown file and parses it
// with the real Mermaid engine (same grammar GitHub renders with), running
// headlessly under jsdom — no browser download needed. Exits non-zero if any
// block fails to parse or if no mermaid blocks are found at all.

import { readFileSync } from 'node:fs';
import { JSDOM } from 'jsdom';

// Mermaid expects a DOM; give it a minimal jsdom one before importing.
const dom = new JSDOM('<!DOCTYPE html><body></body>');
globalThis.window = dom.window;
globalThis.document = dom.window.document;
globalThis.DOMParser = dom.window.DOMParser;

const { default: mermaid } = await import('mermaid');

const files = process.argv.slice(2);
if (files.length === 0) {
  console.error('usage: node check.mjs <file.md> [file.md ...]');
  process.exit(2);
}

const FENCE_RE = /^```mermaid[ \t]*\r?\n([\s\S]*?)^```[ \t]*$/gm;

let blocks = 0;
let failures = 0;

for (const file of files) {
  let text;
  try {
    text = readFileSync(file, 'utf8');
  } catch (err) {
    console.error(`FAIL ${file}: ${err.message}`);
    failures += 1;
    continue;
  }

  let match;
  let index = 0;
  while ((match = FENCE_RE.exec(text)) !== null) {
    index += 1;
    blocks += 1;
    const source = match[1];
    const line = text.slice(0, match.index).split('\n').length;
    try {
      const result = await mermaid.parse(source);
      console.log(`ok   ${file} block ${index} (line ${line}): ${result.diagramType}`);
    } catch (err) {
      failures += 1;
      const message = String(err.message ?? err).split('\n').slice(0, 4).join('\n  ');
      console.error(`FAIL ${file} block ${index} (line ${line}):\n  ${message}`);
    }
  }
}

if (blocks === 0) {
  console.error('FAIL: no ```mermaid blocks found in the given files');
  process.exit(1);
}

if (failures > 0) {
  console.error(`\n${failures} of ${blocks} mermaid block(s) failed to parse`);
  process.exit(1);
}

console.log(`\nall ${blocks} mermaid block(s) parsed OK`);
