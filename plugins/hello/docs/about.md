Hello — sample plugin
=====================

This window is contributed by `plugin.toml`:

    [[contributes.tool-windows]]
    id = "about"
    title = "Hello Notes"
    content = "docs/about.md"
    anchor = "bottom"

A plugin window shows either a text file from its own package (`content`, what
you are reading) or the text one of its commands returns (`command = "word-count"`
for example). It is docked while the plugin is enabled and disappears when the
plugin is disabled or fails.

Commands of this plugin:

Hello: Insert Date Ctrl+Alt+D (or Search Everywhere › Actions)
Hello: Word Count of Active File needs an open file
