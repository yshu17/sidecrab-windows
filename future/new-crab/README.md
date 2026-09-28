# New Crab — future update

A redesigned pet (square flat-topped shell, coral palette) planned to sit
alongside Classic. Nothing here is wired into the app yet; Classic
(`src/frames.js`, `src/sprites.js`) is unchanged.

## Files

| File | What it is |
|---|---|
| `front.png` | FRONT reference |
| `three_quarter.png` | 3/4 reference — turn in-between only, never a resting pose |
| `side.png` | SIDE reference |
| `turn.gif` | FRONT → SIDE → FRONT turn at real size |
| `turn_x6.gif` | the same turn, 6× nearest-neighbour preview |

All three references share one 60×38 canvas:

- floor on the bottom row (y37), alpha strictly 0/255;
- shell centre at x28 in every view, so the turn pivots in place;
- FRONT and 3/4: height 27 / 28, shell 35 px wide (x11..45);
- SIDE is the original approved side design shrunk evenly ×0.75 on both axes
  (53×36 → 40×27, proportions kept; one redundant row/column dropped per ~4,
  no resampling): top y11 and floor y37 exactly like FRONT, shell 33 px wide
  (x12..44).

## States

| State | View | Motion |
|---|---|---|
| IDLE | FRONT | blink, one claw lifts 1 px, back to rest |
| SLEEP | FRONT | crouches, closes eyes, falls asleep |
| NEEDS ATTENTION | FRONT | raises one claw and holds it |
| ERROR | FRONT | short confused reaction, then still |
| THINKING | SIDE | claw lifts (palm 1 px, pincer 2 px), eye looks up 1 px, pause, back |
| CODING | SIDE | rhythmic claw taps at a small keyboard |
| WORKING | SIDE | active legs/claw motion, distinct from CODING |

IDLE and THINKING were prototyped on an earlier, unaligned version of these
references and need to be rebuilt on the files here; the rest are not started.

Every animation is built from copies of its reference frame, changing only the
pixels the motion needs (eyes, one claw, a few dots), and each loop starts and
ends on the unmodified reference frame. No regenerated or redrawn crab.

## Transitions

- Same view as the current state: switch straight to the new state.
- Other view: play the turn first. FRONT → SIDE is 7 frames — front, front
  with eyes +2 px, 3/4 with eyes −1 px, 3/4, 3/4 with eyes +1 px and claw
  +2 px, side with eye −1 px and claw −3 px, side — 60 ms per in-between
  (~300 ms total). SIDE → FRONT plays the same frames in reverse.
- A state change interrupts the current loop immediately, so NEEDS ATTENTION
  and ERROR appear after at most the 300 ms turn; a turn never loops.

## Known gaps

- The silhouette still changes in one step at FRONT → 3/4 and 3/4 → SIDE; a
  truly smooth turn needs two newly drawn in-between angles (~20° and ~65°).
- The eye highlight in `three_quarter.png` (1 px) is a transparent hole; in
  `side.png` it is filled with the light highlight colour.
