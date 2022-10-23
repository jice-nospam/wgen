# Changelog

## [0.3.1] - Unreleased
### Added
- seamless flag on exporter for game engines not supporting multi-texture heightmaps

### Fixed
- changing the height scale in the 3D preview preserves the water level

## [0.3.0] - 2022-10-06
### Added
- editable masks to each step. Makes it possible to apply a step only on some part of the map

### Changed
- improved overall performance and UI responsiveness

### Fixed
- Horizontal rotation in the 3D view

## [0.2.0] - 2022-08-24
### Changed
- improved water erosion algorithm
- thanks to egui 0.19, UI is now responsive and adapts to any resolution
- fbm generator is now multi-threaded and much faster
- export and load/save panels now use a file dialog instead of a simple textbox

### Fixed
- seed is now set correctly when loading a project
- landmass works even if input is not normalized
- 2d and 3d previews work when loading a project with less steps than current project
- hills doesn't crash anymore when using radius variation == 0.0
- worldgen doesn't crash anymore if there is an error while loading/saving a project or exporting a heightmap

## [0.1.0] - 2022-08-05
### Added
- Initial release
- 16 bits grayscale tiled PNG exporter
- save/restore projects to/from [RON](https://github.com/ron-rs/ron) files
- generators :
    - Hills : superposition of hemispheric hills
    - Fbm : fractal brownian motion
    - MidPoint : square-diamond mid-point deplacement
    - Normalize : scale the heightmap to range 0.0..1.0
    - LandMass : scale the terrain so that a defined proportion is above a defined water level. Also applies a x^3 curve above water level to have a nice plain/mountain ratio
    - MudSlide : smoothen the terrain by simulating earth sliding along slopes
    - WaterErosion : carves rivers by simulating rain drops dragging earth along slopes
    - Island : lower the altitude along the borders of the map
- 2D preview :
    - 64x64 to 512x512 grayscale normalized preview (whatever your terrain height, the preview will always range from black to white)
    - possibility to preview the map at any point of the generator by selecting a step
- 3D preview :
    - 3D mesh preview using the same resolution as the 2D preview (from 64x64 to 512x512)
    - skybox (actually a sky cylinder)
    - constrained camera (left click : tilt, right click : pan, middle click : zoom)
    - water plane with user selectable height