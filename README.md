![M.A.X.: Mechanized Assault & Exploration Map Editor](./docs/images/title.png)

# The M.A.X. Map Editor

> **[M.A.X. Map Editor Website](https://suns-echoes.github.io/max-map-editor/)**
>
> **The official M.A.X. Map Editor website.**

> **[M.A.X. Map Editor GitHub Repository](https://github.com/suns-echoes/max-map-editor)**
>
> **The official M.A.X. Map Editor GitHub repository.**

> **[M.A.X. Port](https://klei1984.github.io/max/)**
>
> The M.A.X. Port is an excellent project dedicated to fixing bugs in the
> original game that prevent it from being stable and fully enjoyable.

> **[M.A.X.: Mechanized Assault & Exploration](https://en.wikipedia.org/wiki/Mechanized_Assault_%26_Exploration)**
>
> Wiki about the M.A.X. Game.


▵

## ▰ Introduction

Welcome back M.A.X. Commanders!

I present the announcement of M.A.X. Map Editor – the ultimate open-source tool
for crafting custom maps for the classic M.A.X.: Mechanized Assault & Exploration.

I started this project to fulfill our all-long dreams of assault and exploration
of an unlimited number of new planets and regions where we could try new tactics
and enjoy exploration like it was the first time.

The main goal of this project is to provide an intuitive and enjoyable mapping
experience for all M.A.X. enthusiasts.

with 🖤 for M.A.X. | maXimum map making

### ▰ Planned Features:

Here’s a sneak peek at some of the features planned for the M.A.X. Map Editor:

- build for Linux and Windows with Tauri and WebGL;
- modern and intuitive GUI with quick access to all essential features;
- fluid map zoom and panning with minimap for easy navigation and quick focus on
  specific areas of your map;
- toggleable real-time tiles animation;
- tools for easy and efficient map designing:
    - auto-shore feature,
    - semi-random fill (water, ground, obstacles),
    - terrain templates - no more headaches when drawing mountains!
    - adjacent tile suggestions to boost the design process;
- random map generator with customizable parameters:
    - seed-based generator for easy sharing of awesome maps,
    - the adjustable amount and distribution of water (open ocean, sea, lakes and rivers),
    - the adjustable amount of obstacles like mountains, cliffs, and trees;
- support for custom tile sets and palettes;
- palette editor for tweaking colors;
- palette hot swap for fast choosing the best one;

... only time will tell what other features might be.



[⮝](#)

### ▰ Setting up Linux (Debian / Ubuntu)

> **Tauri v2**
>
> https://v2.tauri.app/start/

#### ▰ Prerequisites

`Ubuntu 22 (or later)` or `Debian 12 (or later)`
`Rust`
`Node.js`


[⮝](#)

## ▰ Building project

#### ▰ Building full application

Run this command in the project root folder using the native terminal
(i.e.: VSCode terminal will not work)

```sh
npm run tauri build
```

The build output directory: `./target/release/bundle`

Generated assets (example):

2. The DEB: `./deb/max-map-editor_X.Y.Z_amd64.deb`


#### ▰ Building frontend only

Run this command in project root folder:

```sh
npm run build
```

The build output directory: `./front/dist/`



[⮝](#)

## ▰ Developing project

### ▰ Development build for full application

Run this command in the project root folder using the native terminal
(i.e., the VSCode terminal will not work):

```sh
npm run tauri dev
```

The above command will build the app in development mode (with access to dev
tools and hot-module-replacements enabled).

Frontend (GUI) will be available at http://localhost:1420/ ¹

> **🛈 NOTE**
>
> ¹ This may or may not work as intended or at all due to missing API provided
>   by the Tauri backend.


### ▰ Development build for frontend only

Run this command in the project root folder:

```sh
npm run dev
```

Frontend (GUI) will be available at http://localhost:1420/ ¹

> **🛈 NOTE**
>
> ¹ This may or may not work as intended or at all due to missing API provided
>   by the Tauri backend.



[⮝](#)

## ▰ Testing project

### ▰ Running unit tests

```sh
npm run test
```

### ▰ Running unit test coverage

```sh
npm run coverage
```



[⮝](#)

## ▰ License

### M.A.X. Map Editor

Licensed under MIT

Copyright © 2024-2025 Aneta Suns


### M.A.X. License

M.A.X. COPYRIGHT © 1996 INTERPLAY PRODUCTIONS. ALL RIGHTS RESERVED.
INTERPLAY PRODUCTIONS IS THE EXCLUSIVE LICENSEE AND DISTRIBUTOR.



[⮝](#)
