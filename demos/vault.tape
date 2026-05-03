Output demos/vault.gif

Set Theme "Catppuccin Mocha"
Set FontSize 14
Set Width 900
Set Height 420
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

Type "# store an API key — encrypted with AES-256-GCM"
Enter
Sleep 400ms

Type "hawk vault set OPENAI_API_KEY sk-proj-abc123"
Sleep 500ms
Enter
Sleep 800ms

Type "hawk vault list"
Sleep 400ms
Enter
Sleep 700ms

Type "hawk vault get OPENAI_API_KEY"
Sleep 400ms
Enter
Sleep 1000ms
