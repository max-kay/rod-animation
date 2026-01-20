# Ride or Die Animator

This crate contains the code for the Ride or Die animations.

## Map

All map data is taken from openstreetmap

https://vector.openstreetmap.org/shortbread_v1/tilejson.json

## Usage
when running the code looks for input files in the `in` directory and renders them to `out`

The animation is rendered to 3840 by 2160 mp4.


## File Format
The file format uses German keywords because it was created for a Swiss German YouTube series.

```
# This is a comment
Animation # keyword `Animation` for animation
# If a single frame is desired use `Bild` German for image

# Animatable Parameters
# in animation mode these parameters can have a start and an end parameter
# separated by a semicolon

# Center
# Either a person indexed by a time or coordinates
Mitte Luca[2T7:30]; (42.3, 3.12) # center at the start and end of the animation
Zoom 11.5
# Time (does not need to be increasing)
Zeit 2T7:30; 2T8:36

Dauer 5.0 # duration of the output animation in seconds

Pins Luca; Clarissa # which pins to use
Pingrösse 400 # height of the pins in pixels
Checkpoints # if present checkpoints will be displayed
```
