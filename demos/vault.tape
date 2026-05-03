Output demos/vault.gif

Set Theme "Catppuccin Mocha"
Set FontSize 14
Set Width 900
Set Height 380
Set Shell "bash"
Set Padding 24
Set PlaybackSpeed 1.0
Set TypingSpeed 55ms

Hide
Type `export MOCK="$PWD/demos/mock_hawk.sh" && alias openhawk='bash "$MOCK"'`
Enter
Sleep 200ms
Show

Type "openhawk vault set OPENAI_API_KEY sk-proj-abc123"
Sleep 450ms
Enter
Sleep 700ms

Type "openhawk vault list"
Sleep 350ms
Enter
Sleep 600ms

Type "openhawk vault get OPENAI_API_KEY"
Sleep 350ms
Enter
Sleep 900ms
