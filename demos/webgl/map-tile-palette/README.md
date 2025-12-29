# The MAP - TILE - PALETTE WebGL demo

## Usage

Follow these steps to use the demo:

1. Download the `demos/webgl/map-tile-palette` directory and open `index.html`
   in an internet browser.
2. Enjoy the demo.
3. Study the demo.

**Note**: The goal of this demo is not to teach WebGL from the beginning but
rather to explain the concept of emulating sprite tiles with palette colors.

▵

## How it works?

### Important files:

| File           | Description
|----------------|--------------------------------------------------------------
| `index.js`     | program entry point, initialization, and rendering
| `webgl.js`     | simple WebGL framework
| `shaders.js`   | GLSL shaders for WebGL
| `helpers.js`   | helper functions (inc. palette color rotation)
| `palettes.js`  | contains an array of 256 RGB colors
| `tiles.js`     | contains an array of tiles data where each tile pixel is index of palette color
| `world-map.js` | contains an array of tile indices for world map cells


### WebGL data

| Data     | GPU format      | Notes
|----------|-----------------|--------------------------------------------------
| tile map | `RGBA texture`  | R and G channels serve as X-Y coordinates to tile texture (B and A skipped)
| tiles    | `ALPHA texture` | alpha channel becomes the X coordinate in the palette texture
| palette  | `RGB texture`   | one row of 256 colors


### How WebGL processes the data

The WebGL 1 rendering context can only operate on textures and buffers,
not data arrays indexed dynamically. This limitation requires uploading
the palette, tiles, and map data to the GPU as textures.
The whole processing occurs in the fragment shader.


#### The fragment shader overview

1. The sampled tile map (world map texture) pixel values reference
   the XY coordinates in the tiles texture:
    - the rounded value (relative to world row and column count) becomes
      the 2D index of the tile (from the tiles texture),
    - the remainder maps the tile coordinates (tile pixels);
2. Each tile pixel value holds reference (index) to the palette texture X coordinate;
3. The final on-screen pixel color is taken from the palette texture.

**Note 1**: Some mathematics is required in the shader because every color
channel is a float value between 0 and 1, and texture coordinates
range from 0 to 1 in both directions.

**Note 2**: Coordinates need to be shifted by half of a pixel width
to prevent situations where math rounding could cause readings
of invalid texture sections.

▵

## ▰ License

### M.A.X. Map Editor

Copyright 2024 Aneta Suns, under ISC license.

▵
