-- Lays out the install window of an `aimer package` disk image: icon view
-- without toolbar or status bar, the generated background, and the app and the
-- Applications shortcut at fixed positions. Finder records all of it in the
-- volume's .DS_Store.
--
-- Usage: osascript dmg_layout.applescript <volume name> <app item name> \
--          <window width> <window height> <app x> <Applications x> <icon y>
on run argv
  set volumeName to item 1 of argv
  set appItem to item 2 of argv
  set winW to (item 3 of argv) as integer
  set winH to (item 4 of argv) as integer
  set appX to (item 5 of argv) as integer
  set linkX to (item 6 of argv) as integer
  set iconY to (item 7 of argv) as integer
  -- Without a GUI session (or when Finder automation is denied) this gives up
  -- with an error instead of waiting for ever.
  with timeout of 60 seconds
    tell application "Finder"
      tell disk volumeName
        open
        set theWindow to container window
        set current view of theWindow to icon view
        set toolbar visible of theWindow to false
        set statusbar visible of theWindow to false
        set the bounds of theWindow to {200, 120, 200 + winW, 120 + winH}
        set opts to the icon view options of theWindow
        set arrangement of opts to not arranged
        set icon size of opts to 128
        set text size of opts to 13
        set background picture of opts to file ".background:background.png"
        set position of item appItem of theWindow to {appX, iconY}
        set position of item "Applications" of theWindow to {linkX, iconY}
        close
        open
        update without registering applications
        -- Give Finder time to write the .DS_Store before the volume is ejected.
        delay 2
        close
      end tell
    end tell
  end timeout
end run
