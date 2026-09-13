# Campaign music

The thin tester plays one looping track per campaign phase. Drop the audio
files here with these exact names (served relative to `index.html`):

| File          | When it plays                                  |
|---------------|------------------------------------------------|
| `lobby.mp3`   | Lobby (ready-up) **and** the victory screen    |
| `level1.mp3`  | Campaign level 1                               |
| `level2.mp3`  | Campaign level 2                               |
| `level3.mp3`  | Campaign level 3                               |

Notes:
- Music is entirely client-side, driven off `campaign.phase` / `campaign.level`
  in the state snapshot. The server knows nothing about audio.
- Tracks are looped and cross-faded when the phase/level changes.
- Playback is unlocked on the first user gesture (browser autoplay policy) and
  can be toggled with the 🔊 button.
- Missing files fail silent — the tester still runs, just without music.
- Format: `.mp3` is assumed. To use a different format, update the `MUSIC` map
  at the top of the `<script>` in `index.html`.
