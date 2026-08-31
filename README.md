# outpost

Talk to a Claude Code session running on another machine.

A session on a remote host is hard to reach on purpose. A console can only
drive sessions it started. The peer-messaging protocol delivers a message
attributed to another *session*, which is wrong when the message is from a
person. Remote Control is outbound HTTPS with no listening socket, so the
vendor's own client is the only thing at the far end.

What is left is the keyboard. `outpost` types into the session's composer over
ssh and tmux, the way a person would, and reads the conversation back out of
the session's own transcript.

    outpost status            what the session says it is doing, and its windows
    outpost read [n]          the last n exchanges, both sides
    outpost pane [n]          the window itself, with n lines of scrollback
    outpost send <text>       type it and press Enter; `-` reads stdin

Point it with two environment variables. Neither has a default, so the tool
carries no record of which machines exist:

    OUTPOST_HOST      an ssh destination (a `Host` block in ~/.ssh/config)
    OUTPOST_WINDOW    a tmux target, `session:window`

## Two things it is careful about

**A send is not reported until the session has actually taken it.** Reporting
success when the keystrokes are accepted would mean a green tick for "bytes
reached a pipe". Instead it types, checks the text is really on screen *before*
pressing Enter, and then waits for the session's transcript to record the turn.
A message typed while the session is working is queued rather than read, and it
says so instead of claiming delivery.

**It has four verbs and no escape hatch.** Driving a remote container by hand
means an ssh session with the process table, the caches and the logs all in
reach. If the intent is to talk to the session rather than go and look around,
then the tool should not be able to go and look around either.

## Building

    cargo build --release

The transcript parsing comes from the `reader` crate in
[memview](https://github.com/xinutec/memview) as a pinned git dependency,
rather than copied in — it owns several rules about what counts as a human turn
that are easy to get subtly wrong.
