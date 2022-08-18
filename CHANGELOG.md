# Changelog

## [Unreleased]
### Fixed
- seed is now set correctly when loading a project
- landmass works even if input is not normalized

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