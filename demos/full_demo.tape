Output demos/openhawk_demo.gif

Set Theme "Catppuccin Mocha"
Set FontSize 13
Set Width 1000
Set Height 580
Set Shell "bash"
Set Padding 24
Set PlaybackSpeed 1.0
Set TypingSpeed 50ms

Hide
Type `export MOCK="$PWD/demos/mock_hawk.sh" && alias openhawk='bash "$MOCK"'`
Enter
Sleep 200ms
Show

# vault
Type "openhawk vault set OPENAI_API_KEY sk-proj-abc123"
Sleep 450ms
Enter
Sleep 700ms

Type "openhawk vault list"
Sleep 350ms
Enter
Sleep 600ms

# agents
Type `openhawk run "python research_agent.py"`
Sleep 450ms
Enter
Sleep 700ms

Type "openhawk ps"
Sleep 350ms
Enter
Sleep 650ms

# orchestrate
Type `openhawk orchestrate "research quantum computing and write a summary then review it"`
Sleep 500ms
Enter
Sleep 3000ms

# undo + verify
Type "openhawk undo"
Sleep 350ms
Enter
Sleep 800ms

Type "openhawk verify sess-abc123"
Sleep 350ms
Enter
Sleep 1000ms
