# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project

`tile-documents` is a Rust project aimed at replacing PDFs with a better document format.

## Project Definition

This project is a Tauri v2 application with the following characteristics:

- It works like a typical PDF viewer, albeit on a different file format (tiles).
- It must cross-compile for macOS, Linux, Windows, Android, and iOS.
- It must register as a handler with the OS for files with extension `*.tile` or media type
  `application/tile`. When such a file is opened through typical OS means, this app is the
  one that gets to open the file (at least if it has been installed).
- Tile files are CAR containers as defined in the Web Tiles specification at https://dasl.ing/tiles.html.
- A good Rust implementation of CAR is `iroh-car`, a good example of using CAR in code is 
  https://github.com/blacksky-algorithms/rsky/blob/main/rsky-satnav/.
- The way in which a tile file is opened is as follows:
  1. The file is in CAR format, the CAR is parsed. The MASL from the CAR header is extracted and the `resources` entry in
     the MASL is the resource map. Keys in the resource map are /-rooted paths into the tile and values contain both
     the CID of each resource under `src` and HTTP headers. The CID of each resource corresponds to an entry in the CAR.
     Keep track of the byte offset of the data of each entry keyed by CID so that entries in the resource map can be loaded
     by seeking into the CAR and reading just that entry.
  2. The Tauri backend exposes the content of that tile using a `tile:` custom protocol in which the authority is derived from
     the file name and the path is the key into `resources`.
  3. The Tauri frontend receives a message to show the tile and sets up to show the tile in the UI. The message also contains
     the full MASL metadata from the CAR header.
  4. The frontend is coded in JavaScript using custom elements built with the lit framework and reactive data sources driven
     by https://www.npmjs.com/package/refrakt.
  5. The UI is tab-based. Showing a new tile involves creating a new tab and then creating an `iframe` inside of that tab
     that points to the `tile:` URI. This triggers loading the content from the custom protocol.
  6. Each tab has the first icon from the `icons` field of the MASL and a text label from the `name` field of the MASL. It
     also has a close button.

## Commands

Once the Rust project is initialized (`Cargo.toml` present), standard commands apply:

```sh
cargo build          # build
cargo test           # run all tests
cargo test <name>    # run a single test by name
cargo clippy         # lint
cargo fmt            # format
```

## License

Apache 2.0
