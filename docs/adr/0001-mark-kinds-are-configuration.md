# Mark kinds are configuration, not code

I wanted to tell work workspaces apart from personal ones. The obvious path was a
second hardcoded command and key beside the existing mark, which would have put
the word "personal" in `src/`. Instead the config declares an ordered list of
`mark-kind` nodes, each with a name, a key and a command, and the app knows a
kind only by those three things — so a reader looking for the feature will grep
for "personal" and find it in `niri-groom.kdl` and in a shell script, never in
the source.

The kinds are semantically exclusive: a workspace is work or personal, not both.
The app deliberately does not enforce that. It owns no mark store and does not
know the kinds are related, so exclusivity lives in the toggle script, which owns
every store and clears the other kind when it sets one. Adding enforcement to the
app would mean teaching it which kinds exclude which, which is exactly the
knowledge this decision keeps out.
