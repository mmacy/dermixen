# Security

Report a security problem in Dermixen privately through [GitHub's private vulnerability reporting](https://github.com/mmacy/dermixen/security/advisories/new) rather than in a public issue. Say which command, window action, or file format triggers the problem and how to reproduce it.

The app reads audio files, project files, and library files from disk and writes renders and library files. No crate in the workspace opens a network connection, so most problems worth reporting are in how the app decodes a file it was given.
