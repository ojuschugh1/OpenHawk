Output demos/agents.gif

Set Theme "Catppuccin Mocha"
Set FontSize 14
Set Width 900
Set Height 460
Set Shell "bash"
Set Padding 20
Set PlaybackSpeed 1.0
Set TypingSpeed 60ms

Hide
Type `export MOCK="$PWD/demos/mock_hawk.sh"`
Enter
Type `alias hawk='bash "$MOCK"'`
Enter
Show

Type "# spawn an agent — any command, any language"
Enter
Sleep 400ms

Type `hawk run "python research_agent.py"`
Sleep 500ms
Enter
Sleep 800ms

Type "# live CPU and memory via sysinfo"
Enter
Sleep 300ms

Type "hawk ps"
Sleep 400ms
Enter
Sleep 800ms

Type "# agent corrupted a file? roll back instantly"
Enter
Sleep 300ms

Type "hawk undo"
Sleep 400ms
Enter
Sleep 900ms

Type "# verify the agent did what it claimed"
Enter
Sleep 300ms

Type "hawk verify sess-abc123"
Sleep 400ms
Enter
Sleep 1000ms
