# Helpers shared by the entrypoint scripts. Sourced, not executed.

# seed_into <src> <dst>: copy tracked seed files an empty volume would hide.
# `cp -Rn` never overwrites and exits 0 when it skips, so operator edits and
# restored backups survive while a genuine failure still stops the container.
seed_into() {
    [ -d "$1" ] || return 0
    mkdir -p "$2"
    cp -Rn "$1/." "$2/"
}

# own_volume <dir>: chown a mounted volume to openmmo, but only when its root
# is not already owned — volumes can hold gigabytes, and a recursive chown on
# every start would delay startup for nothing.
own_volume() {
    [ "$(stat -c %u "$1")" = "$(id -u openmmo)" ] || chown -R openmmo:openmmo "$1"
}
