# Urchin User Guide

[Back to the README](README.md)

- [Starting Urchin](#starting-urchin)
- [Mouse](#mouse)
- [Keys](#keys)
- [Menus](#menus)
- [Views](#views)
- [Minimalist View](#minimalist-view)
- [While a song plays](#while-a-song-plays)
- [Opening folders](#opening-folders)
- [Your own song order](#your-own-song-order)
- [Ticking several rows](#ticking-several-rows)
- [Resizing panes](#resizing-panes)
- [Search](#search)
- [Library](#library)
- [Queue](#queue)
- [Playlists](#playlists)
- [Shuffle Albums](#shuffle-albums)
- [Continue Into Next Folder](#continue-into-next-folder)
- [Equalizer](#equalizer)
- [Your own presets](#your-own-presets)
- [EQ follows moved music](#eq-follows-moved-music)
- [Crossfade](#crossfade)
- [ReplayGain](#replaygain)
- [Themes](#themes)
- [Visualizers](#visualizers)
- [Show in File Manager and Copy Path](#show-in-file-manager-and-copy-path)
- [Speakers, headphones and media keys](#speakers-headphones-and-media-keys)
- [Where Urchin keeps things](#where-urchin-keeps-things)

## Starting Urchin

```sh
urchin            # opens your music folder
urchin ~/Music    # or any folder
```

Your music folder is the one set with Settings › Music Folder…, or your
system's Music folder until you set one. `urchin --help` and
`urchin --version` do what you'd expect.

## Mouse

- **Click** a folder or song to highlight it; a folder opens in the right pane.
- **Double-click** a folder to play everything in it, or a song to play from it.
- **Middle-click** plays.
- **Right-click** a row for its menu (Shuffle, Play Next, Add to Queue, Add to
  Playlist, Open, Pin, Show in Finder, Copy Path…).
- **Wheel** scrolls the pane under the pointer. On the volume bar it moves the
  volume 1%, on the seek bar 5 seconds.
- **Drag** the seek bar to scrub and the volume bar to set the volume.
  Right-click the volume bar to mute and unmute.
- **◀ ▶** go up out of this folder and back down into the highlighted one.
  Click any part of the path to jump there.
- **⇄ Shuffle** shuffles the highlighted folder (or what's ticked), and
  **★ Pin Folder** pins it to the File menu.
- Click **EQ** for the equalizer.

## Keys

| Action | Keys |
|---|---|
| Move | Arrows, `h` `j` `k` `l`, or `Ctrl+B` `N` `P` `F` |
| Page / top / bottom | `Page Up` `Page Down`, `Home` `End` |
| Open folder / up a level | `→` or `l` / `←` or `h` |
| Back through folders you visited | `Backspace` |
| Move between controls | `Tab` / `Shift+Tab` (`Enter` presses) |
| Play | `Enter` |
| Pause | `Space` or `p` (`p` twice quickly restarts the song) |
| Stop | `s` |
| Next / previous song | `n` / `b` |
| Seek back / forward 10 seconds | `,` / `.` |
| Volume (1% steps) | `+` / `-` |
| Right-click menu | `m`, the Menu key, or `Shift+F10` |
| Search | `/` |
| Library | `i` |
| Queue view | `Shift+W` |
| Add to Queue / Play Next | `Shift+Q` / `Shift+N` |
| Next Album | `Shift+A` |
| Add to Playlist | `a` |
| Move a song in its folder | `Shift+↑` / `Shift+↓` (or `K` / `J`) |
| Tick a row | `x` (`X` for a run, `Esc` to clear) |
| Resize panes | `Shift+←` `→` or `<` `>`; `Alt+↑` `↓` or `Alt+K` `J` |
| Open a folder | `Ctrl+O` |
| Equalizer | `Ctrl+E` |
| Themes | `Ctrl+T` |
| Menus | `F10` or `Alt`+letter |
| Help | `F1` |
| Quit | `q` or `Ctrl+Q` |

`F1` (Help › Keyboard & Mouse) has the full list inside Urchin.

## Menus

`F10` or `Alt`+letter opens them; `←` `→` move between them.

- **File:** Open Folder…, New Playlist…, Add New Playlist…, your pinned
  folders (☆), your last ten folders, Clear Recent Folders, Quit
- **Playback:** Playlists, Queue, Search…, Library; Play/Pause, Next, Next
  Album, Previous, Stop; Forward and Back 10 Seconds, Volume Up and Down;
  Repeat (plays the song again), Repeat All (goes back to the first song after
  the last), Shuffle Albums, Continue Into Next Folder; Shuffle Queue, Clear
  Queue, Save Queue as Playlist…
- **Settings:** Equalizer…, Equalizer On, Crossfade, ReplayGain; Theme…,
  Standard View, Flipped View, Minimalist View; the Visualizers; Music
  Folder…, Show Hidden Folders
- **Help:** Keyboard & Mouse, About Urchin Player

## Views

- **Standard View** (the default): the player and visualizer at the top, the
  folders below, the ◀ ▶ path row at the bottom.
- **Flipped View:** the path row at the top, then the folders and the
  visualizer, with the player at the bottom.
- **Minimalist View:** see below. It works with either of the other two.

## Minimalist View

Settings › Minimalist View hides the menu bar row and draws the panes without
frames, titles only. Rest the mouse on an empty part of the top row and the
menu bar slides out (move off the row and it goes away again), or press `F10`
or `Alt`+letter. Dividers still drag the same way; a thin line appears under
the pointer to show where you've grabbed.

The row with ◀ ▶, the path, EQ, Shuffle and Pin Folder is hidden when you
start Urchin; press any key and it comes out. 5 seconds into a song you
picked, it goes away along with the other folders, leaving just the music. Any
folder navigation key (`←` `→`, `h` `l`, `Backspace`, `Tab`) brings it back.
Moving the mouse on its row shows it too, for 3 seconds after the mouse stops.

## While a song plays

5 seconds into a song you started, the browser fills the window with that
song's folder. Moving up and down in it (arrows, `j` `k`, `Page Up`/`Down`,
`Home`/`End`) keeps it that way, and so do the playback keys (`Space`, `p`,
`n`, `b`, `+` `-`…). Going somewhere else brings the two panes back: `←` `→`,
`h` `l`, `Backspace`, `Tab`, `Enter` on a folder, the ◀ ▶ buttons or the path.

If you are browsing the folder that's playing and move up or down, it goes to
one pane right away instead of waiting.

If you navigate away while the same song keeps playing, one pane (and in
Minimalist View the empty toolbar row) comes back 5 seconds after your last
key or click. A new song, a restart or a different song cancels it.

A title too long for its space scrolls slowly from right to left.

## Opening folders

File › Open Folder… (`Ctrl+O`) browses your drives and folders. `→` opens a
folder, `←` or `Backspace` goes up, and `Tab`, `/` or `~` lets you type a path
(`Tab` completes it). Open shows the folder in Urchin; Play and Shuffle play it
straight away.

The File menu lists the last ten folders you opened, and folders you pinned
(★ Pin Folder, or Pin / Unpin on a folder's right-click menu) above them.
Settings › Music Folder… sets the folder Urchin starts in (Use This Folder),
and Settings › Show Hidden Folders shows folders whose names start with a dot.

## Your own song order

A folder's songs are in name order until you change that. Drag a song up or
down with the mouse, or press `Shift+↑` / `Shift+↓` (or `K` / `J`, capital
letters, where there are no arrow keys) to move the highlighted song one place.
Right-click a song (or press `m`) for Move Up and Move Down, and for Reset
Order, which puts the folder back in name order.

Urchin remembers the order for each folder in `folder_order.json` next to its
settings (nothing is written into your music folders), and playing the folder,
or a folder above it, follows it. Songs added later come after the ones you
placed. Moving a song changes the next time you play the folder; it does not
touch what is already in the queue.

Only songs move: the folders stay where they are, and the Library, Search and
playlists keep their own order. Renaming or moving a folder starts it over in
name order.

## Ticking several rows

Click a row's first two cells (its tick column) to tick it, or drag down the
tick column to tick a run of rows. From the keyboard, `x` ticks the highlighted
row and moves down, `X` ticks everything from the last tick to here, and `Esc`
unticks them all (`Ctrl+X` in the Search box, where letters are typed).
Ctrl-click and Shift-click work too in terminals that pass them on.

While rows are ticked, Play, Shuffle, Play Next, Add to Queue, Add to Playlist,
Remove (Queue) and Move Up/Down act on all of them, and the right-click menu
says how many and offers Clear Ticks.

Ticking works in the folder browser (songs and folders), the Queue, a
playlist's songs, the Library's songs and Search results. Search keeps ticks
while you change what's typed, so you can gather songs found by different
words.

## Resizing panes

Drag the lines where two panes meet to make one wider, or drag the edge
between the panes and the visualizer to make it taller or shorter. The line
lights up under the pointer, and in terminals that support it (kitty, Ghostty
and foot, for example) the pointer turns into resize arrows. Double-click a
divider to put it back.

From the keyboard: `Shift+←` / `Shift+→` (or `<` / `>` without arrow keys)
move the divider beside the highlighted pane, and `Alt+↑` / `Alt+↓` (or
`Alt+K` / `Alt+J`) move the visualizer's edge. Sizes are remembered.

## Search

Playback › Search…, or `/`. Finds songs anywhere in your music folder as you
type. Type any part of the artist, album or song name, in any order:
`pink floyd` lists every Pink Floyd song, and `pink floyd shine` or
`pink floyd diamond` finds "Pink Floyd - Shine on you crazy diamond".
Capitals, accents and apostrophes don't matter (`beyonce` finds Beyoncé,
`dont stop` finds Don't Stop). Every word has to be somewhere in the song's
name or in the folders it is in (the file extension doesn't count).

Each song is one line with its folders at the right
(`Rock › Pink Floyd › The Wall`), and the highlighted one's whole trail is
shown under the list; the same song in two folders is two results.

- `↑` `↓`, `Page Up` `Page Down`, `Home` and `End` move through the results.
- `Enter`, a double-click or the Play button plays the songs found, starting
  with the highlighted one.
- Right-click a result (or press `→`) for Play, Play Next, Add to Queue, Add to
  Playlist, Show in Folder (opens its folder in Urchin), Show in Finder and
  Copy Path.
- `Esc` closes.

The folders are read again each time the box opens, so new songs are found.
Opened from the Library view, Search looks at tags instead (see below).

## Library

Playback › Library, or `i`. Your songs by their tags rather than their folders.
Genre, Artist and Album sit side by side, and each narrows the next (pick Rock
and the artists and albums are Rock ones). Every column starts with an "All"
row; a song with a missing tag is listed under Unknown Genre, Artist or Album,
last. A song's artist is its Album Artist tag when it has one, so a
compilation stays one album.

- `→` (or `l`) moves to the next column, and from an album (or the ▸ at the end
  of its row) shows the songs as one pane, in disc and track order.
- `←` (or `h`) moves back a column. `Backspace` or the ◀ button goes back from
  the songs to the columns, and from the columns to the folders; `Esc` returns
  to the folders.
- `Enter`, or a double-click, plays everything under the highlighted row (from
  the highlighted song, in the songs pane).
- Right-click or `m` for Shuffle, Play Next, Add to Queue, Show Songs (on an
  album) and, on a song, Add to Playlist, Show in Folder, Show in Finder and
  Copy Path.
- `/` opens Search over the tags: every word has to be in a song's title,
  artist, album or genre, and each result shows `Genre › Artist › Album`.

The tags are read the first time (on another thread, with a count on screen)
and kept in `library.json` next to Urchin's settings; after that only new or
changed files are read again. On a narrow window two columns show, then one.

## Queue

What plays, in order. Whatever folder or playlist you start is the queue.
Playing another folder or playlist replaces it.

Right-click a song, folder or playlist (or press `m`) and choose Add to Queue
(`Shift+Q`) to put it at the end, or Play Next (`Shift+N`) to put it right
after the song playing; several Play Next songs play in the order you added
them. If nothing is playing, they start playing.

Playback › Queue (`Shift+W`) shows the queue in one pane: the song playing has
a ▶, and songs already played are dimmed.

- `Enter` or a double-click plays a song; `Delete` takes it out.
- `Shift+↑` `↓` (or `K` / `J` where there are no arrow keys) or a drag with the
  mouse moves it.
- `a` adds it to a playlist.
- `z` (or the Shuffle button) mixes the songs still to come; the song playing
  and the ones already played stay where they are.
- `Shift+C` clears the queue except for the song playing (with nothing
  playing, it empties it).
- `Shift+S` asks for a name and saves the queue as a playlist.
- `Esc`, `Backspace` or `Shift+W` goes back.

Shuffle, Clear and Save are in the Playback menu and the right-click menu too.

The queue is kept between runs, in `queue.m3u8` next to Urchin's settings.
When you start Urchin again it is there in the Queue view, paused on the song
you were listening to, at the place you left it: `Space` (or ▶) carries on
from there, and `n`, `b` and seeking move that place first if you like. It's
remembered when the song changes, when you pause or stop, and when you quit or
close the terminal. A song you left at its very end comes back as the next
one, from its start. Songs that have been moved or deleted are left out (if
the song you were on is one of them, the next one takes its place).

## Playlists

File › New Playlist… asks for a name and makes an empty playlist in
`playlists/` next to Urchin's settings.

File › Add New Playlist… reads `.m3u` and `.m3u8` files. Click a playlist (or
press `Space`) to put a ✓ on it, and `Enter` (or Import) imports every checked
one; double-click imports just that one. Imports are copied into `playlists/`
next to Urchin's settings, with every song's full path, so they keep working
when the original moves. Entries Urchin can't play are skipped: streams
(`http://…`), missing files, other playlists, and songs on another machine
(`file://server/…`, or `\\server\share\…` on Windows; map the share to a drive
letter instead).

To add a song to a playlist, right-click it in the folders, the Library, Search
or the Queue (or press `a`), choose Add to Playlist, then click a playlist.

Playback › Playlists shows your playlists on the left and the selected one's
songs on the right: `Enter` or a double-click plays, `←` `→` switch panes, and
`Esc` or `Backspace` goes back to the folders. Right-click a playlist (or press
`m`) for Shuffle, Play Next, Add to Queue, Rename, Delete, Show in Finder and
Copy Path; `F2` renames and `Delete` deletes from the keyboard. With no
playlists yet, the pane says so and points to File › Add New Playlist.

## Shuffle Albums

Playback › Shuffle Albums makes every Shuffle (a folder, a playlist, the
Library, the Shuffle button, Shuffle Queue) mix whole albums instead of single
songs: the albums come in a random order and each one plays start to finish,
in its usual order.

Songs with Album and Album Artist tags make one album wherever their files
are; songs without tags go by folder, so the folder is the album. A disc
folder (`CD1`, `Disc 2`…) counts as part of the album above it. `Shift+A`
(Playback › Next Album) skips the rest of the album playing.

## Continue Into Next Folder

In the Playback menu, off until you turn it on. When the folder you played
reaches its last song, the next folder beside it follows on with no gap, and
after the last one there, the folder after its parent, and so on up through
your music folder, like a radio working through a genre. Folders with nothing
to play are skipped.

The songs are added to the end of the queue as the last one starts (a message
says what's next), so the Queue view shows them. A folder you shuffled carries
on shuffled. It does nothing with Repeat or Repeat All on, or for a queue from
a playlist, the Library or Search.

## Equalizer

Settings › Equalizer…, `Ctrl+E`, or click EQ. Shapes the song that's playing:
a preamp and ten bands, ±20 dB. Pick one of 33 presets (VLC's 18 and 15 more,
from Bass booster to Vocal) with the Preset button or `[` / `]`.

In the equalizer:

- `←` `→` (or `h` `l`) pick a band, `↑` `↓` (or `k` `j`) move it half a dB,
  `Page Up` `Page Down` 3 dB, and `0` sets it to 0. Click or drag a bar, or
  use the wheel on it, with the mouse.
- `r` (or Reset) makes the song follow its folder's preset again.
- `e` (or the EQ: On/Off button) turns the equalizer on and off, the same as
  Settings › Equalizer On.
- `Tab` moves between the bars and the Preset, Reset, EQ and Close buttons.
- `Space`, `p`, `n`, `b` and `Shift+A` still work, so you can skip songs while
  you tune.
- `Esc`, `q` or Close closes it.

A preset you pick also becomes its folder's preset: every other song directly
in that folder (not subfolders) plays with it, unless it has its own EQ.
Adjust any band and that song keeps its exact sound ("R&B (customized)"); it's
saved as you go. Reset makes a song follow its folder's preset again; picking
Flat clears the folder's preset.

## Your own presets

Shape a song's sound, then press `s` in the equalizer (or Preset › Save as
Preset…) and give it a name. Your presets are listed first in the Preset list,
under Your Presets, and work like the built-in ones: pick one for a song and
its folder follows.

Saving again under the same name updates the preset, and every folder using
it follows, as do songs that had it unchanged (songs you adjusted afterward
keep their own sound). Right-click one of yours in the list (or press `m` on
it) for Rename… and Delete, or press `F2` or `Delete`. Deleting a preset puts
the folders that used it, and songs that had it unchanged, back to flat.
They are kept in `equalizer_presets.json` next to Urchin's settings.

## EQ follows moved music

A song with its own EQ, and a folder with a preset, are also recognized by
their sound (a fingerprint of the first two seconds, after any silence).
Rename or move a file or a folder in Finder, Explorer or the terminal, and the
next time it plays Urchin finds its EQ again and says so.

The fingerprints are kept in Urchin's EQ files (`equalizer.json`,
`equalizer_folders.json`); nothing is written into your music. A song on a
drive or share that isn't connected keeps its EQ for when it's back. A song
converted to another format counts as a different song.

## Crossfade

Settings › Crossfade: Off (the default: songs join with no gap), or 2, 4, 6, 8,
10 or 12 seconds. When a song ends by itself, its last seconds fade out as the
next one fades in (equal power, so the level holds steady). Skipping, picking a
song or seeking cuts straight over. Each song keeps its own EQ; the new one's
takes over halfway through the fade.

## ReplayGain

Settings › ReplayGain: Off (the default), Track (every song at the same
loudness) or Album (every album at the same loudness, with the songs in it as
loud as each other as mastered). It reads the ReplayGain tags your tagger
wrote (FLAC, Ogg, MP3, M4A, and Opus's R128 tags with the `ffmpeg` feature)
and uses the peak tags so a raised song never clips. Songs without tags play
as they are; nothing is analyzed or written to your files.

## Themes

Settings › Theme…, or `Ctrl+T`. All the iTerm2-Color-Schemes themes, HaX0R
Blue to start with. `↑` `↓` (or a click) try one out on the whole window as you go,
typing narrows the list by name, `Enter` keeps it and `Esc` goes back to the
one you had.

## Visualizers

Pick one in the Settings menu under Visualizers; pick the checked one again to
hide the visualizer and give its room to the folders. Every visualizer freezes
where it is when you pause, and stops when you stop or a speaker or headphones
go away.

### Spectrum analyzer (Stereo)

The default. A vintage hi-fi style segmented LED display of the music as it
plays, 20 Hz to 16 kHz with frequency labels underneath, taking about a third
of the window. Its colors come from your theme: each bar is the theme's
accent, and wherever it reaches, its top two segments are darker shades of it.
It reads the audio after the equalizer and before the volume control, so EQ
changes show up and the volume knob doesn't shrink it.

### Other visualizers

- **Bokeh:** out-of-focus circles of light on black, big ones for the bass and
  small ones for the treble, moving with the tempo. The circles follow your
  theme.
- **Bokeh Light:** the same on your theme's light background, with its darker
  colors for the circles.
- **Club Lights:** colored spotlights swinging over a dark sequin wall, one beam
  per band, one swing per bar.
- **Galaxy:** a starfield over nebula clouds, the stars glowing with the music,
  and the odd shooting star.
- **Lines:** vertical level meters across the window, bass in the middle and
  treble at the edges, in two of your theme's colors.
- **Synthwave:** a half-set sun, mountains and a grid road running to the
  horizon.
- **SynthMoonWave:** Synthwave at night: a moon instead of the sun, a dark
  midnight-blue landscape, a starry sky and the odd shooting star falling into
  the moon or behind the mountains.
- **Trace:** thin needles mirrored up and down from the center, with streaks
  that linger where the music just was.

### Scopes

Oscilloscope, Vectorscope and Spectroscope are
[scope-tui](https://github.com/alemidev/scope-tui) by alemi, running the
scope-tui way inside Urchin, fed with exactly what Urchin is playing (after
the equalizer) and colored by your theme. All three are the trace alone: no
header, no labels, no grid, so the folders get the rows the text used to take,
and the Vectorscope, a row bigger than the rest, gets the picture to itself.

Click the scope (or `Tab` to it and press `Enter`) to give it the keyboard,
then use scope-tui's keys:

- All three: `↑` `↓` zoom, `←` `→` samples, `s` scatter, `r` reference lines,
  `Tab` next scope, `Esc` reset. The wheel on the scope zooms too.
- Oscilloscope: `u` tunes it to the note playing so it stands still, `t`
  trigger, `e` edge, `p` peaks, `-` `=` trigger depth, `Page Up` `Page Down`
  threshold.
- Vectorscope: `g` goniometer.
- Spectroscope: `w` window, `l` log, `p` phase difference, `Page Up`
  `Page Down` averaging.

`q` or a click elsewhere gives the keyboard back to Urchin.

## Show in File Manager and Copy Path

On the right-click menu (or `m`) of any song or folder: in the folder browser,
the Queue, a playlist, the Library and Search.

**Show** opens the folder in your file manager with the song selected. The menu
calls it after your file manager:

- **macOS:** Show in Finder
- **Windows:** Show in Explorer
- **Linux and the BSDs:** Show in File Manager, for whichever file manager
  answers the freedesktop "show items" call (Nautilus, Dolphin, Thunar, Nemo,
  Caja), on X11 or Wayland. If none does, the folder is opened with `xdg-open`
  instead, without the song highlighted (that's what you get with PCManFM on a
  Raspberry Pi's desktop). On Wayland the window may open behind your terminal
  (GNOME keeps the focus where it is).
- **WSL:** the folder opens in Windows' File Explorer.
- **No desktop** (logged in over SSH, or a console): the path is copied instead.

If you rename or fix a song (or folder) there, Urchin shows the new name
within a second and keeps it highlighted. It only watches after Show, and
stops when you come back to the terminal (or after ten minutes).

**Copy Path** puts the full path on the clipboard through your terminal, so
over SSH it lands on the computer you're typing on (iTerm2, Ghostty, kitty,
WezTerm, Alacritty, Windows Terminal; tmux needs `set -g set-clipboard on`).
The message shows it too.

## Speakers, headphones and media keys

Urchin plays through your system's sound output and follows it:

- Turn on a Bluetooth speaker or headphones while music plays and Urchin moves
  over to it once your system makes it the output.
- Pick another output in your system's sound settings and Urchin moves to it.
- When the output goes away (a speaker turned off, headphones unplugged or out
  of range), the music pauses where it was and stays paused. Your system falls
  back to the built-in speakers, but nothing plays on them until you press
  play.

On macOS and Linux, the play/pause, next and previous keys on your keyboard,
headphones and Bluetooth speakers control Urchin, and the song shows in Now
Playing (Control Center on macOS, MPRIS on Linux). Volume buttons on a speaker
change the system volume, not Urchin's. There are no media keys on Windows yet.

## Where Urchin keeps things

Everything Urchin remembers is in its settings folder, never in your music
folders:

- **macOS:** `~/Library/Application Support/urchin/`
- **Linux and the BSDs:** `~/.config/urchin/`
- **Windows:** `%APPDATA%\urchin\`

(`XDG_CONFIG_HOME` moves it on any system.) In it: `settings.json`, the EQ
files (`equalizer.json`, `equalizer_folders.json`, `equalizer_presets.json`),
`folder_order.json`, `library.json`, `queue.m3u8` and the `playlists/` folder.

If you ever need to report a skip or a stop, `URCHIN_TRACE=1 urchin` writes a
debug log, `trace.log`, to the same folder.
