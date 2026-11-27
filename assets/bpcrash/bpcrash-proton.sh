#!/usr/bin/env bash
# Steam launch option: bash "/absolute/path/to/bpcrash-proton.sh" %command%
# Keep the normal Mintfixed one-shot injection. Run native triage only after
# Steam's original command returns. Do not start a persistent Wine watcher.
set -u
here=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P) || exit 1
if (( $# == 0 )); then
    echo 'bpcrash-proton: missing Steam %command%' >&2
    exit 2
fi
if [[ -e "$here/disabled" || ! -x "$here/bpcrash-watch" || ! -f "$here/bpcrash-triage.exe" ]]; then
    exec "$@"
fi
# Preserve Steam's runtime/container command prefix. Only alter the Proton
# subcommand, and discard the original game executable and game arguments.
args=("$@")
runner=()
for (( i=0; i+1<${#args[@]}; i++ )); do
    if [[ ${args[i]##*/} == proton && ( ${args[i+1]} == waitforexitandrun || ${args[i+1]} == run ) ]]; then
        runner=("${args[@]:0:i+1}" run)
        break
    fi
done
if (( ${#runner[@]} == 0 )); then
    echo 'bpcrash-proton: Proton command not recognized; starting game without post-exit triage' >&2
    exec "$@"
fi
session=$(mktemp "$here/mintfixed-session.XXXXXXXX") || exec "$@"
# mktemp creates a private file. Export only the basename to the game hook.
trap 'rm -f -- "$session"' EXIT
export MINTFIXED_BPCRASH_SESSION=${session##*/}
"$@"
game_status=$?
unset MINTFIXED_BPCRASH_SESSION
if [[ -s "$session" ]]; then
    timeout --kill-after=5s 120s "$here/bpcrash-watch" --triage-only "$session" "$here/bpcrash-triage.exe" "${runner[@]}" \
        >> "$here/bpcrash-post-exit.log" 2>&1 || echo 'bpcrash-proton: post-exit triage failed; see bpcrash-post-exit.log' >&2
fi
exit "$game_status"
