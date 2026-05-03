Output demos/agents.gif

Set Theme "Catppuccin Mocha"
Set FontSize 14
Set Width 900
Set Height 460
Set Shell "bash"
Set Padding 24
Set PlaybackSpeed 1.0
Set TypingSpeed 55ms

Hide
Type `export MOCK="$PWD/demos/mock_hawk.sh" && alias openhawk='bash "$MOCK"'`
Enter
Sleep 200ms
Show

Type `openhawk run "python research_agent.py"`
Sleep 450ms
Enter
Sleep 700ms

Type "openhawk ps"
Sleep 350ms
Enter
Sleep 700ms

Type "openhawk undo"
Sleep 350ms
Enter
Sleep 800ms

Type "openhawk verify sess-abc123"
Sleep 350ms
Enter
Sleep 1000ms
