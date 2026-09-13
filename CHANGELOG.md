# Changelog

## [0.4.1] - Unreleased

### Added

- ThermalErosion generator: crumbles slopes steeper than a talus threshold into scree, mass
  conserving, resolution independent (runs on a working grid like WaterErosion)
- FluvialErosion generator: carves a dendritic valley network with the stream-power law, talus crumbling between incisions, optional uplift

### Changed

- Better, faster and resolution independant WaterErosion
- MidPoint is resolution independant
- MudSlide is kept for existing projects; its hover text and the README now point at ThermalErosion
- LandMass, Island, MudSlide, Hills and Fbm run on all cores through one shared row-parallel helper;
  same output as before

### Fixed

- project files saved by a previous version can be loaded again (any older version loads, a newer
  one is refused with a message); the three `ex_*.wgen` examples are re-saved in the new format
- moving a step up recomputes the steps it passed over
- a mask edit is stored on its step at the end of each brush stroke and recomputed once when
  leaving the mask editor; a preview refresh no longer closes the mask editor
- crashes when editing the step list (add, delete, clear, enable, resize the window) while the
  generator is still computing, and when editing steps during an export
- a panic inside a generator or the exporter now shows an error popup instead of killing the app
  (or leaving the export panel disabled forever)
- the previews refresh as soon as a step is computed, without waiting for mouse input
- resizing the window no longer recomputes the terrain
- LandMass produced NaN heights at land proportion 0 or 1
- export tile size and tile count are now integers with a valid range
- memory leak in the 3D preview (one uv buffer per regeneration)
- exporting a project with a masked step panicked when the export was taller than wide, and
  squashed the mask vertically when wider than tall
- panic `The absolute aspect ratio cannot be zero` when moving or resizing the window
- Island progress bar used the map width instead of its height on non-square exports
- WaterErosion parameters were measured in pixels, so an export looked nothing like the preview;
  drops lost all their speed on the slightest climb; the erosion brush was applied half a cell off
  its own position
- MidPoint left a 1-px dark seam on the right and bottom edges at every preview size, grid seams
  at non-power-of-two export sizes and a wrong octave on non-square exports

### Changed

- project files contain only the seed and the steps, pretty-printed (masks stay on one line)
- the mask button shows which step's mask is being edited
- the export computes the stack on a single heightmap instead of one per step (memory ÷ steps)
- hills are generated row by row (faster at export sizes)
- log timestamps share one clock across threads
- editing a step, the seed or Clear while the generator is computing interrupts the running step
  instead of waiting for it to finish (export is unaffected)
- WaterErosion runs on a working grid of at most its `resolution` setting and scales the result up,
  so the export gets the erosion seen in the preview and takes seconds instead of minutes at large
  sizes; existing projects will look slightly different

## [0.4.0] - 2025-03-16

### Changed

- exports to single channel EXR (slightly smaller files)
- upgraded to egui 0.29, three_d 0.18

## [0.3.1] - 2022-10-25

### Added

- seamless flag on exporter for game engines not supporting multi-texture heightmaps
- now you can export to either 16 bits PNG (preferred format for Unreal Engine) or 16 bits float OpenExr format (for Godot)
- added shore height parameter to landmass generator to avoid z fighting issues between the land mesh and a water plane

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
