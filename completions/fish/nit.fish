function __nit_using_command
    set -l words (commandline -opc)
    test (count $words) -ge 2; and test "$words[2]" = "$argv[1]"
end

complete -c nit -f -a '-init' -d 'Create a workspace'
complete -c nit -f -a '-migrate' -d 'Migrate legacy storage'
complete -c nit -f -a '-assign-ids' -d 'Assign missing IDs'
complete -c nit -f -a '-migrate-timeless' -d 'Migrate legacy timed IDs'
complete -c nit -f -a '-drive-create' -d 'Create a NIT Drive'
complete -c nit -f -a '-drive-migrate' -d 'Copy Plain storage into a NIT Drive'
complete -c nit -f -a '-unlock' -d 'Unlock a Vault session'
complete -c nit -f -a '-lock' -d 'Lock the Vault session'
complete -c nit -f -a '-session-status' -d 'Show Vault session state'
complete -c nit -f -a '-ai-roadmap' -d 'Generate an AI Roadmap'
complete -c nit -f -a '-root' -d 'Print workspace root'
complete -c nit -f -a '-path' -d 'Print storage path'
complete -c nit -f -a '-status' -d 'Show workspace statistics'
complete -c nit -f -a '-search' -d 'Search entries'
complete -c nit -f -a '-ls' -d 'List entries'
complete -c nit -f -a '-show' -d 'Show an entry'
complete -c nit -f -a '-edit' -d 'Edit an entry'
complete -c nit -f -a '-archive' -d 'Archive an entry'
complete -c nit -f -a '-import' -d 'Import entries'
complete -c nit -f -a '-completions' -d 'Generate shell completions'
complete -c nit -f -a '-tui' -d 'Open the TUI'
complete -c nit -f -a '-help' -d 'Show help'
complete -c nit -f -a '-version' -d 'Show version'

complete -c nit -f -a '-si' -d 'Short-term Idea'
complete -c nit -f -a '-mi' -d 'Medium-term Idea'
complete -c nit -f -a '-li' -d 'Long-term Idea'
complete -c nit -f -a '-st' -d 'Short-term To-do'
complete -c nit -f -a '-mt' -d 'Medium-term To-do'
complete -c nit -f -a '-lt' -d 'Long-term To-do'
complete -c nit -f -a '-n' -d 'Note'
complete -c nit -f -a '-x' -d 'Item'

complete -c nit -n '__nit_using_command -init' -f -a '--private --tracked'
complete -c nit -n '__nit_using_command -ls' -f -a '--archived'
complete -c nit -n '__nit_using_command -search' -f -a '--archived --all'
complete -c nit -n '__nit_using_command -show; or __nit_using_command -edit' -f -a '(nit -completion-ids 2>/dev/null) --archived'
complete -c nit -n '__nit_using_command -archive; or __nit_using_command -ai-roadmap' -f -a '(nit -completion-ids 2>/dev/null)'
complete -c nit -n '__nit_using_command -completions' -f -a 'bash zsh fish'
complete -c nit -n '__nit_using_command -import' -F
complete -c nit -n '__nit_using_command -drive-migrate' -F
complete -c nit -n '__nit_using_command -unlock' -F
complete -c nit -n '__nit_using_command -drive-create' -f -a '--dry-run --initialize'
