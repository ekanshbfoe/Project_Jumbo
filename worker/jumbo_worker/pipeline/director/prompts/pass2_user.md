Now direct the camera for the narration you just wrote. The panels of each line are fixed; for each line, in order, choose:
- "preset": the camera move that serves the moment. A line with one panel takes a single-panel move; a line with two or three panels takes split_screen, floating_panels or horizontal_slices.
- "emotion": the feeling of the line.
- "voice": the narrator's "tone" (a short tag such as neutral, warm, tense, playful, grim, whispered), "pace" (slow, normal, fast) and "intensity" (0 to 1).
- "transition_in": how this line's panels arrive: cut, fade, slide or wipe, with a duration in seconds.

Start with "notes": one or two sentences on the visual arc of the scene.

Camera moves:
- hold: static panel; quiet exposition, a pause, a dialogue-heavy beat.
- zoom {from, to, focus [x, y] in 0..1}: push in on a face or detail (to > from), or pull out to reveal (to < from).
- pan {direction left|right|up|down, distance 0-0.5}: slow drift; establishing shots, wide or tall panels.
- shake {intensity 0-60, frequency 1-30}: rumbling, explosions, sustained tension.
- action_tilt {angle -20..20, punch 1-1.5}: fast punch-in with a tilt; hits, reveals, sudden action.
- kinetic_jitter {intensity 0-60, decay 0-1}: violent jitter; impacts, shock, screams.
- panoramic_fade {direction, fade 0-0.5}: wide pan with fades; scene changes, flashbacks, time skips.
- hud_popup {text 1-120 chars, position top|center|bottom}: game-style text box; system messages, signs, captions.
- split_screen {orientation vertical|horizontal, stagger 0-2}: 2-3 panels; confrontations, parallel reactions.
- floating_panels {tilt 0-12, drift 0-15, period 3-30}: 2-3 tall panels; introductions, line-ups, reflection.
- horizontal_slices {drift 0-0.2, lead left|right}: 2-3 wide short panels; close-ups, eye contact, stare-downs.

Vary the moves the way a good editor would: calm stretches stay calm, and the strong moves are saved for the moments that earn them.

Answer with the JSON object only.
