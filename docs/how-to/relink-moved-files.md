# Relink moved files

A mix document names each track by its path and by the hash of its bytes. When the files move, the hash is how the app finds them again.

## Recognize a mix whose files have moved

```
dermixen render set.dmx set.wav
```

```
error: cannot read /Volumes/old/07 L.S.C. - Big Brain.mp3, which set.dmx names: No such file or directory (os error 2). Run dermixen mix relink set.dmx to point the mix at the file wherever it is now.
```

`render` and `play` hash every file before they start and refuse the mix until every file is where the document says and contains the bytes the document names. You don't lose anything: the document is untouched, and the fix is one command. In the window, the lane of such a track says `the file is missing`, `the file has changed`, or `the file cannot be read` in place of its waveform, and the status line names the tracks.

## Relink from the library

```
dermixen mix relink set.dmx
```

Every track's file is hashed. A track whose file is in place is kept. Every other track is looked up by its hash in [the library](../library.md), and when the library names a path that contains those bytes, the document is pointed at it. The window runs the same search when it opens a mix, so a mix whose files moved after a scan opens relinked, with the status line saying which tracks were. A scan after the move is what puts the new paths in the library.

## Relink from folders

```
dermixen mix relink set.dmx --under /Volumes/new/goa --under ~/Downloads
```

When the library has no record of the new paths (the files were moved and no scan has run since), `--under` names folders to search, and may be repeated. Each folder is searched in the order given, hashing every audio file in it until one matches. The output says what happened to each track:

```
  1. relinked /Volumes/new/goa/comp/VA - Fill Your Head with Phantasm Vol 2/07 L.S.C. - Big Brain.mp3  was /Volumes/old/07 L.S.C. - Big Brain.mp3
  2. kept     /Volumes/new/goa/comp/VA - Mind Rewind [DMRCD01]/202 Total Eclipse & Simon Posford - Sound Is Solid (Remix).mp3
```

A track found nowhere is reported as `missing`, with where it was looked for, and the command still exits with code 0, since the report is the point. If you know where the file is, run the command again with that folder under `--under`. The document is rewritten only when a track was relinked, and the command points the library's record for each relinked track at its new file too.

## Files that can't be found

Relinking finds a file by its bytes. A file that was re-encoded, retagged, or otherwise rewritten has different bytes and is never found. For such a track, add the new file to the mix with `mix add --position` at the old track's place, then remove the old track in the window with the delete key. The new file is analyzed as any new track is, so the old track's anchors and nodes don't transfer to it.

A track whose file is at the path the document names but can't be read, as an unmounted volume or a missing permission leaves it, is reported with the operating system's reason rather than searched for, because mounting the volume or fixing the permission is the remedy.
