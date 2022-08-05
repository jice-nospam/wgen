# WGEN - a simple heightmap generator

There are a lot of great terrain generators out there but most of them have a free version with a terrain size limitation.

This is a much simpler generator, but it can export maps as big as you want.

Continent example, using a high frequency FBM :
![continent example](https://raw.githubusercontent.com/jice-nospam/wgen/main/doc/ex_continent.jpg)

Island example, using mid-point deplacement algorithm :
![island example](https://raw.githubusercontent.com/jice-nospam/wgen/main/doc/ex_island.jpg)

Smoother landscape using only hills generator :
![hills example](https://raw.githubusercontent.com/jice-nospam/wgen/main/doc/ex_hills.jpg)

Exemple of (untextured) 4K x 4K landscape imported in Unreal Engine 5 :
![UE5 example](https://raw.githubusercontent.com/jice-nospam/wgen/main/doc/ex_ue5.jpg)

If you like this project and want to support its development, feel free to donate at [![Donate](https://img.shields.io/badge/Donate-PayPal-green.svg)](https://paypal.me/guldendraak)

# Manual
## Generators
This is where you control the world generation. You can stack several "generators" that applies some modification to the heightmap.
Select the generator with the dropdown button, then press `New step` button to add it to the stack.
You can click on a step label in the stack to select it and display its parameters. Click the `Refresh` button once you changed the parameters values to recompute the heightmap from this step.

![Generators UI](https://raw.githubusercontent.com/jice-nospam/wgen/main/doc/ui_gen.jpg)

The current version features those generators :
- Hills : superposition of hemispheric hills to generate a smooth terrain
- Fbm : fractal brownian motion can be used to add noise to an existing terrain or as first step to generate a continent-like terrain.
- MidPoint : square-diamond mid-point deplacement generates a realistic looking heightmap
- Normalize : scales the heightmap back to the range 0.0..1.0. Some generators work better with a normalized heightmap. Check your heightmap values range in the 2D preview.
- LandMass : scale the terrain so that a defined proportion is above a defined water level. Also applies a x^3 curve above water level to have a nice plain/mountain ratio
- MudSlide : smoothen the terrain by simulating earth sliding along slopes
- WaterErosion : carves rivers by simulating rain drops dragging earth along slopes
- Island : lower the altitude along the borders of the map

## Terrain preview
You have a 2D preview displaying the heightmap (at current selected step in the generators UI). You can change the preview heightmap size from 64x64 for very fast computation to 512x512 for a more precise visualization. If `live preview` button is checked, the 2D preview will be updated at every step during computation.

![3D preview UI](https://raw.githubusercontent.com/jice-nospam/wgen/main/doc/ui_2d.jpg)

You also have a 3D preview displaying the final 3D mesh. The mesh uses the same resolution as the 2D preview.
You can change the view by dragging the mouse cursor in the view :
- rotate the terrain with left button
- zoom with middle button
- pan with right button

You can also display a water plane with configurable height and a grid to help visualize the terrain.

![3D preview UI](https://raw.githubusercontent.com/jice-nospam/wgen/main/doc/ui_3d.jpg)

## Save/Load project
Here you can save the current generator configuration (all the steps with their parameters) in a plain text file using RON format. You can also load a previously saved project, erasing the current configuration.

![Save project UI](https://raw.githubusercontent.com/jice-nospam/wgen/main/doc/ui_project.jpg)
