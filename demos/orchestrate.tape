Output demos/orchestrate.gif

Set Theme "Catppuccin Mocha"
Set FontSize 14
Set Width 900
Set Height 520
Set Shell "bash"
Set Padding 20
Set PlaybackSpeed 1.0
Set TypingSpeed 60ms

Hide
Type `export MOCK="$PWD/demos/mock_hawk.sh"`
Enter
Type `alias openhawk='bash "$MOCK"'`
Enter
Show

Type "# decompose a task — parallel with 'and', sequential with 'then'"
Enter
Sleep 400ms

Type `openhawk orchestrate "research quantum computing and write a summary then review it"`
Sleep 500ms
Enter
Sleep 3200ms
