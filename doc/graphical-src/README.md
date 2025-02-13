# Graphical source files

This directory contains source files for graphical assets used in the documentation, etc.

## `nixops4-source-60deg-grid-vertical.svg`

This is the primary source file for the logo, created with Inkscape on a 60-degree grid.
Inkscape only supports a grid that includes vertical lines, whereas for a Nix lambda-inspired logo,
we need a grid that includes horizontal lines.

## `nixops4-horiz-sq.svg`

This file is derived from `nixops4-source-60deg-grid-vertical.svg` by rotating it 30 degrees, cropping it, and making the document size square, aligning the logo within the square.

## `nixops4.svg`

This file is an optimized export of `nixops4-horiz-sq.svg` for use in documentation, etc.

- Inkscape:
  - File -> Export...
  - Select "Plain SVG" as the format
  - Name: `nixops4-raw.svg`
- `nix shell nixpkgs#nodePackages.svgo`
  - `svgo -i nixops4-raw.svg -o nixops4.svg`
  - `rsvg-convert --output nixops4-128.png -w 128 -h 128 nixops4.svg`
