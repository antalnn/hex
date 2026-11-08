# Hex

Hex is local voice dictation for Mac, Windows and Linux.

## About this fork

This is essentially the same application with the same functionality, plus a
few added features:

| Feature | What it does |
| --- | --- |
| Local model server | Hex starts its own llama.cpp server on this machine and uses it for dictation cleanup and Rewrite Select. |
| Cloud models | Cloud models live in the app's Models tab. Cloud was reachable before but not inside the menu; each feature now picks Local or Remote on its own. |
| Rewrite Select | Press the shortcut, speak an instruction, and the selected text is replaced with the model's result. Windows only. |
| Excluded apps | Name the apps where Hex's shortcuts do nothing, so a game keeps its own keys. Windows only. |
| Escape cancels a recording | Escape discards the current recording. |
| Wrap code terms | Vocabulary terms you mark as code paste wrapped in backticks. |
| Paste where dictation started | Hex returns to the window you were in when the shortcut went down, even if you alt-tabbed away mid-dictation. Windows only. |
