# The GIF the README opens with, as a script rather than a binary nobody can
# regenerate.
#
#   bash tools/demo/build-store.sh
#   vhs tools/demo/loop.tape
#
# `bash <script>` rather than `./<script>`: the scripts here carry no execute
# bit — none of the four in `scripts/` do either — and Windows has no notion of
# one to carry. Spelling the interpreter works the same on all three platforms,
# and this project is developed on the one where `./` does not.
#
# Every command below runs against the store that script builds, and what
# appears is what the binary printed. `jq` renders the block because these
# commands answer in JSON — they are written for an agent to read, and the
# whole point of the recording is to show a person what the agent is handed.
#
# `LETEO_SYSTEM_LANGUAGE=en` because Leteo speaks the language of the machine
# it runs on, and this recording goes in an English README.

Output ../../assets/leteo-loop.gif

Set Shell bash
Set FontSize 15
Set Width 1180
Set Height 720
Set Padding 18
Set TypingSpeed 22ms
Set Theme { "name": "leteo", "background": "#12121a", "foreground": "#d8d8e0", "cursor": "#9a86fd", "black": "#12121a", "red": "#f07178", "green": "#6ad4a0", "yellow": "#ffcb6b", "blue": "#82aaff", "magenta": "#c792ea", "cyan": "#89ddff", "white": "#d8d8e0", "brightBlack": "#5c6370", "brightRed": "#f07178", "brightGreen": "#6ad4a0", "brightYellow": "#ffcb6b", "brightBlue": "#82aaff", "brightMagenta": "#c792ea", "brightCyan": "#89ddff", "brightWhite": "#ffffff" }

Hide
Type "cd store/payments" Enter
Type "export LETEO_DATABASE=$PWD/../demo.db LETEO_SYSTEM_LANGUAGE=en" Enter
Type "export PS1='$ '" Enter
Type "clear" Enter
Sleep 1s
Show

Sleep 500ms
Type "# An agent worked here yesterday. Nobody asked it to remember."
Sleep 900ms
Enter
Sleep 500ms

Type "# A new session opens. This is what it gets, unprompted:"
Sleep 500ms
Enter
Sleep 400ms

Type "leteo hook session-start | jq -r .hookSpecificOutput.additionalContext"
Sleep 400ms
Enter
Sleep 5s

Type "clear"
Enter
Sleep 400ms

Type "# And the one line you see:"
Sleep 700ms
Enter
Type "leteo hook session-start | jq -r .systemMessage"
Sleep 500ms
Enter
Sleep 1800ms

Type "# Mid-task it asks the store, not you:"
Sleep 900ms
Enter
Type "leteo search webhooks | jq -r '.[].title'"
Sleep 400ms
Enter
Sleep 2200ms

Type "leteo search 'rounding money' | jq -r '.[].title'"
Sleep 400ms
Enter
Sleep 2500ms

Type "# One binary. One SQLite file. Nothing left the machine."
Sleep 1800ms
Enter
Sleep 2s
