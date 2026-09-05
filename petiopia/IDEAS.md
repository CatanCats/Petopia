# Weaving mini-games into Kitten Café

Right now every mini-game (Feather Chase, Yarn Maze, Rockpool Fishing, Laser
Dash, Purr Rhythm) lives behind one door: the **Games** tab. You pick a game,
play it in a fullscreen modal, get coins/mood/bond, and go back to the room.
That's fine, but it means the mini-games never touch the rest of the
simulation — they're a side activity, not part of caring for the café.

Implemented as a first example: the **Homeward Maze** (see `awayMode` in
Settings). It isn't in the Games tab at all — it's triggered by the world
(a neglected cat going missing) and its outcome changes real state (the cat
comes home). That's the pattern the ideas below extend.

## Ideas, roughly cheapest → most involved

1. **Contextual triggers instead of a menu.** Tap-and-hold a toy already
   placed in the room (a scratching post, a ball pit) to start that toy's
   mini-game directly on the stage, with the cat that's nearest to it —
   no sheet, no menu, just play where you see it. The Games tab stays as a
   fallback/browse view for when no cat is nearby.

2. **A cat asks, rather than you choosing.** When a cat's mood dips and its
   favourite game is off cooldown, show a small in-room prompt bubble (like
   the tummy-rub glow) — tapping it launches that cat's favourite game
   immediately, tying "the games I unlock" to "the cat's stated want".

3. **Minigame results feed the room, not just the cat.** A perfect Purr
   Rhythm run could nudge every cat in the room's mood, not just the
   soloist's — turns a solo activity into a shared café moment, and gives
   multi-cat rooms a reason to play together.

4. **Difficulty and skin tied to breed/trait**, already partly true via
   `stageIndex` scaling — extend it so an "athletic" cat's Laser Dash has a
   faster dot but a bigger score reward, making trait choice matter for
   which games are worth playing with which cat.

5. **Seasonal/limited mini-games as room events**, not shop purchases: a
   snow day spawns a one-off "catch the snowflakes" mode directly on the
   stage for 24 hours, reusing the existing ambient-particle system rather
   than a separate canvas game.

6. **More "rescue"-style stakes-based games** like Homeward Maze: a dirty
   litter box that's been ignored too long could spawn a quick "chase the
   mess before it spreads" mini-game instead of a plain tap-to-clean, so
   neglect has a mini-game *consequence* as well as a mini-game *reward*.

7. **A featured/rotating mini-game with a bonus multiplier**, shown as a
   badge on the stage itself (not just the Games list), so there's always
   one game that's worth seeking out that day — cheap to add on top of the
   existing cooldown system.

The common thread: pull the trigger and the payoff out of the Games tab and
into the room simulation itself, so playing a mini-game is something that
*happens* while caring for the café, not a separate errand.
