Output demos/openhawk_demo.gif

Set Theme "Catppuccin Mocha"
Set FontSize 13
Set Width 1000
Set Height 620
Set Shell "bash"
Set Padding 20
Set PlaybackSpeed 1.0
Set TypingSpeed 55ms

Hide
Type `export MOCK="$PWD/demos/mock_hawk.sh"`
Enter
Type `alias hawk='bash "$MOCK"'`
Enter
Show

# ── VAULT ─────────────────────────────────────────────────────────────────────
Type "# store an API key (AES-256-GCM encrypted)"
Enter
Sleep 300ms
Type "hawk vault set OPENAI_API_KEY sk-proj-abc123"
Sleep 450ms
Enter
Sleep 750ms

Type "hawk vault list"
Sleep 350ms
Enter
Sleep 650ms

# ── SPAWN + PS ────────────────────────────────────────────────────────────────
Type "# spawn a research agent"
Enter
Sleep 300ms
Type `hawk run "python research_agent.py"`
Sleep 450ms
Enter
Sleep 750ms

Type "hawk ps"
Sleep 350ms
Enter
Sleep 700ms

# ── ORCHESTRATE ───────────────────────────────────────────────────────────────
Type "# decompose a complex task across agents"
Enter
Sleep 300ms
Type `hawk orchestrate "research quantum computing and write a summary then review it"`
Sleep 500ms
Enter
Sleep 3000ms

# ── UNDO ──────────────────────────────────────────────────────────────────────
Type "# agent corrupted a file? roll back instantly"
Enter
Sleep 300ms
Type "hawk undo"
Sleep 400ms
Enter
Sleep 850ms

# ── VERIFY ────────────────────────────────────────────────────────────────────
Type "# verify the agent actually did what it claimed"
Enter
Sleep 300ms
Type "hawk verify sess-abc123"
Sleep 400ms
Enter
Sleep 1100ms
