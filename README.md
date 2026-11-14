# blite: The Boring Local Information Telling Engine

## Overview

A self-contained tool (written in Rust) that tells you things you might want to know, using information you already have — like all that YAML in your repository, or those docs that someone wrote, back when that was a thing humans remembered how to do.

## Why does this exist?

Because a repository tells a story, if you know how to read it.

The longer the history, the richer the tale.

Things are the way they are for reasons, and those reasons are knowable.

Careful consideration and lessons learned shaped what you see before you.

There was _intention_ and maybe even a vision, a hope for a better world.

If you don't know the story, it might seem like a wild claim.

But who do you ask, when everyone who could share that story has moved on?

Maybe you take it upon yourself to do as they once did, and read the docs.

_But what to read?_

_Where to start?_

_There's so much!_

_Surely... too much._

Too much for any one person to keep in their head.

Nobody could know all these things.

Best ask the machine to figure out what's worth looking at.

## How do I obtain this majestic tool?

```sh
cargo install --locked blite
```

## What does it do?

That depends on how far along things are when you install it.

There's a whole roadmap in the [docs/roadmap](./docs/roadmap) directory.

At the very least, it should be able to:

- index thousands of Kubernetes manifests in hundreds of milliseconds.
- run queries against that data in tens of milliseconds.
- direct you to the relevant files, ranked by relevance to your query.
- provide deterministic output with modest memory usage.
- all without making your machine get all toasty.
- without requiring a Python installation, container engine, external vector database, or a fancy GPU.

If the winds of implementation have been favourable, it may (optionally) do more than that by the time you install it!

The best way to find out what your version does is to ask it for help!

```sh
blite --help
```

## License

```text
SPDX-License-Identifier: MIT OR Apache-2.0
```

`blite` is available under [your choice](https://fossa.com/blog/dual-licensing-models-explained/) of _either_ the [MIT License](https://colstrom.mit-license.org) _or_ the [Apache License, Version 2.0](https://www.apache.org/licenses/LICENSE-2.0).

See [`LICENSE-MIT`](LICENSE-MIT) and [`LICENSE-APACHE`](LICENSE-APACHE) at the root of the repository for the full text of each.

Both are written in fancy lawyer-speak. If you prefer more down-to-earth language, consider the following:

- tl;drLegal has simple visual summaries available: [`MIT`](https://www.tldrlegal.com/license/mit-license) or [`Apache-2.0`](https://www.tldrlegal.com/license/apache-license-2-0-apache-2-0)
- FOSSA has more in-depth overviews available: [`MIT`](https://fossa.com/blog/open-source-licenses-101-mit-license/) or [`Apache-2.0`](https://fossa.com/blog/open-source-licenses-101-apache-license-2-0/)
