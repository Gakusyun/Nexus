"""Sample the screenshot so we can talk about colours instead of impressions."""

import sys

from PIL import Image

path = sys.argv[1]
image = Image.open(path).convert("RGB")
print(f"{image.size[0]}x{image.size[1]}")

points = [
    ("window backdrop (below the strip)", 800, 210),
    ("command strip, far left", 60, 110),
    ("command strip, far right", 1560, 110),
    ("inside the input, left of the text", 150, 128),
    ("input border, left", 88, 128),
    ("inside the input, right of the text", 1240, 128),
    ("gap between input and the purple button", 1280, 128),
    ("purple button", 1350, 128),
    ("advanced button", 1480, 128),
    ("title bar", 700, 30),
    ("filter chip", 100, 238),
    ("list background", 800, 400),
    ("empty-state plate", 800, 610),
]

for label, x, y in points:
    print(f"{label:44} ({x:4},{y:4}) = {image.getpixel((x, y))}")

# A row straight through the middle of the command bar, to see where the bands change.
print("\nrow y=110 transitions:")
previous = None
for x in range(0, image.size[0]):
    colour = image.getpixel((x, 110))
    if previous is None or max(abs(a - b) for a, b in zip(colour, previous)) > 6:
        print(f"  x={x:5} {colour}")
        previous = colour
