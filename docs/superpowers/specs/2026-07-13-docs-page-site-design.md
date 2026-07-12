# Design: docs.page narrative docs site for mnesis (#223)

**Date:** 2026-07-13
**Issue:** #223 — *docs: host mnesis narrative docs the kameo way (docs.page)*
**Milestone:** 1 — Mnesis: Pre-Freeze (1.0 blockers)
**Status:** Approved (pending spec review)

## Goal

Stand up a **zero-build** narrative documentation site for mnesis on
[docs.page](https://docs.page) (Invertase), reachable at:

```
https://docs.page/devrandom-labs/mnesis
```

docs.page renders `.mdx` pages + a `docs.json` config **straight from the GitHub
repo** — no CI, no static-site build, no cost. Because it is zero-build, nothing
is added to `nix flake check` and the docs ship the moment the files land on
`main`.

This first pass delivers a **go-live skeleton**: the config, the full sidebar
structure (frozen), **4 fully-written pages**, and short stubs for the rest so no
sidebar entry is a dead end. Prose for the remaining pages follows in later PRs.

### Non-goals (explicit)

- **API reference docs** — stay on `docs.rs/mnesis` (separate track, automatic on
  publish). This site is *narrative* docs only.
- **Custom domain** (e.g. `docs.mnesis.dev`) — optional post-freeze polish; a
  CNAME swap that needs no content change. The free `docs.page/...` URL is the
  target here.
- **Google Analytics** — field left out; add a key later if wanted.
- **Filling the 14 stub pages** — follow-up PRs.
- **A bespoke `socialPreview` OG image** — needs a proper 1200×630 PNG that does
  not yet exist (see Assets).

## Audience & ordering

One site that reads **top-to-bottom**, concepts first, framed by the **reader's
journey** (sidebar organization "B" from brainstorming): a newcomer can walk the
whole thing ("why event sourcing" → "I shipped an adapter"); an integrator can
jump to the back half. The journey framing was chosen over mirroring the crate
graph because the group names describe *what the reader is doing*, not *which
crate the code lives in*.

## Files added

```
docs.json                       # docs.page config: branding + sidebar
docs/
  logo.svg                      # devrandom mark (from ../web brand assets)
  index.mdx                     # Introduction                     (REAL)
  getting-started.mdx           # Quickstart                       (REAL)
  concepts/
    event-sourcing.mdx          # stub
    ddd.mdx                     # stub
    cqrs.mdx                    # stub
    hexagonal.mdx               # stub
    closing-the-books.mdx       # Closing the Books                (REAL)
  model-a-domain/
    aggregates.mdx              # stub
    handle-decide.mdx           # stub
    sagas.mdx                   # stub
  persist-events/
    envelopes-codecs.mdx        # stub
    repository.mdx              # stub
    snapshots.mdx               # stub
    backup-restore.mdx          # stub
  go-live/
    subscriptions.mdx           # stub
    adapters.mdx                # stub
    writing-a-store-adapter.mdx # stub
  reference/
    stability.mdx               # Stability & the 1.0 promise      (REAL)
```

> Note: `docs/` already holds unrelated `.org`/`.md` working files and a
> `superpowers/` tree. docs.page only reads `docs.json` + the `.mdx` files it
> references via the sidebar, so the existing files are inert and left untouched.

## `docs.json` shape

```jsonc
{
  "name": "Mnesis",
  "description": "Event sourcing for Rust — no Box<dyn>, no runtime downcasting, no hidden allocations.",
  "logo": {
    "light": "https://github.com/devrandom-labs/mnesis/raw/refs/heads/main/docs/logo.svg",
    "dark":  "https://github.com/devrandom-labs/mnesis/raw/refs/heads/main/docs/logo.svg"
  },
  // "socialPreview": TODO — needs a 1200x630 PNG (follow-up)
  // "scripts": { "googleAnalytics": "G-XXXX" } — omitted for now
  "sidebar": [
    { "group": "Getting Started", "pages": [
      { "title": "Introduction",  "href": "/",                "icon": "globe"  },
      { "title": "Quickstart",    "href": "/getting-started", "icon": "rocket" }
    ]},
    { "group": "Learn Event Sourcing", "pages": [
      { "title": "Event Sourcing",    "href": "/concepts/event-sourcing",    "icon": "clock-rotate-left" },
      { "title": "Domain-Driven Design", "href": "/concepts/ddd",            "icon": "diagram-project"   },
      { "title": "CQRS",              "href": "/concepts/cqrs",              "icon": "code-branch"       },
      { "title": "Hexagonal Architecture", "href": "/concepts/hexagonal",   "icon": "cube"              },
      { "title": "Closing the Books", "href": "/concepts/closing-the-books", "icon": "book"              }
    ]},
    { "group": "Model a Domain", "pages": [
      { "title": "Aggregates",      "href": "/model-a-domain/aggregates",   "icon": "layer-group" },
      { "title": "Handle & Decide", "href": "/model-a-domain/handle-decide","icon": "gavel"       },
      { "title": "Sagas",           "href": "/model-a-domain/sagas",        "icon": "sitemap"     }
    ]},
    { "group": "Persist Events", "pages": [
      { "title": "Envelopes & Codecs", "href": "/persist-events/envelopes-codecs", "icon": "box"          },
      { "title": "Repository",         "href": "/persist-events/repository",       "icon": "database"     },
      { "title": "Snapshots",          "href": "/persist-events/snapshots",        "icon": "camera"       },
      { "title": "Backup & Restore",   "href": "/persist-events/backup-restore",   "icon": "floppy-disk"  }
    ]},
    { "group": "Go Live", "pages": [
      { "title": "Subscriptions",          "href": "/go-live/subscriptions",          "icon": "tower-broadcast" },
      { "title": "Adapters",               "href": "/go-live/adapters",               "icon": "plug"            },
      { "title": "Writing a Store Adapter","href": "/go-live/writing-a-store-adapter","icon": "screwdriver-wrench" }
    ]},
    { "group": "Reference", "pages": [
      { "title": "Stability & the 1.0 Promise", "href": "/reference/stability", "icon": "shield-halved" },
      { "title": "API Docs (docs.rs)", "href": "https://docs.rs/mnesis", "icon": "rust" }
    ]}
  ]
}
```

Icons follow kameo's FontAwesome convention. The final "API Docs" entry is an
external link to docs.rs, keeping the reference track one click away without
duplicating it here.

## The 4 real pages

Each is adapted from **existing in-repo prose** — no net-new authoring, just
reformat to `.mdx` — and each exercises a different rendering feature (headings,
fenced Rust code, blockquotes/callouts), proving the pipeline works before the
stubs get filled.

1. **`index.mdx` — Introduction** (`/`). What mnesis is, the
   no-`Box<dyn>`/zero-copy pitch, and the crate map (kernel → store → adapters,
   with the wake crates in core). *Source: `README.md` + the CLAUDE.md project
   overview.*
2. **`getting-started.mdx` — Quickstart** (`/getting-started`). The bank-account
   domain end to end, copy-pasteable. *Source: `examples/inmemory` +
   the `README.md` snippet.*
3. **`concepts/closing-the-books.mdx` — Closing the Books**. The
   snapshot-alternative modeling discipline. *Source: the module docs in
   `crates/mnesis/src/closing_the_books.rs` (already prose — reformat to `.mdx`,
   keep the Dudycz/Kurrent citations).*
4. **`reference/stability.mdx` — Stability & the 1.0 promise**. *Source:
   `STABILITY.md`, near-verbatim.*

## Stubs

The remaining 14 pages are each **one short paragraph** — "This page is coming
soon." — plus a link to the most relevant `docs.rs` item or `examples/` crate, so
a stub still hands the reader somewhere useful. This keeps the sidebar whole and
the structure frozen without blocking go-live on 14 pieces of prose.

## Assets

Branding comes from the devrandom web repo at `../web`:

- **Logo:** copy `../web/devrandom/public/brand/devrandom-mark-rainbow.svg` →
  `docs/logo.svg`. The rainbow mark reads on both light and dark backgrounds, so
  one file serves `logo.light` and `logo.dark`. Referenced by its raw GitHub URL
  (docs.page fetches over HTTP — a local path will not resolve).
- **socialPreview:** deferred. OG/Twitter cards want a 1200×630 **PNG**; SVG does
  not render there and no correctly-sized asset exists yet. Field left commented
  with a TODO. (Interim fallback if wanted: the 512×512 chrome icon, though the
  aspect ratio is wrong.)

> The logo is a *devrandom org* mark, not a mnesis-specific wordmark. That is
> acceptable for go-live; a dedicated mnesis logo can replace it later with a one
> line change.

## Verification

There is **no build step**, so verification is mechanical and local:

1. `docs.json` parses as valid JSON (e.g. `jq . docs.json`).
2. Every internal `href` in the sidebar maps to a `docs/<href>.mdx` file that
   exists (relative `/` → `docs/index.mdx`, `/x/y` → `docs/x/y.mdx`). Checked with
   a small script that walks the sidebar and stats each file.
3. External `href`s (docs.rs) are well-formed URLs, not local paths.
4. **Docs-only, no gate impact:** the change touches no `Cargo.toml`,
   `flake.nix`, `rust-toolchain.toml`, or source under `crates/`/`adapters/` — so
   `nix flake check` behavior is unchanged. Confirmed by the file list above.
5. After merge, load `https://docs.page/devrandom-labs/mnesis` and confirm the
   sidebar renders and the 4 real pages display (post-merge, manual — docs.page
   reads `main`).

## Out of scope / follow-ups

- Custom domain (`docs.mnesis.dev`) + `socialPreview` OG image.
- Google Analytics key.
- Filling the 14 stub pages (one follow-up card, or incremental PRs).
- A mnesis-specific logo/wordmark to replace the devrandom org mark.
