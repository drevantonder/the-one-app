# Sourced by the Mac's and iPhone's throwaway.sh: one folder of what cargo compiles, for every checkout's copy here, so
# a copy's first build compiles only this repo's own crates (its program stays in the copy). disk.sh's sweep deletes it
# when the disk runs low.
export CARGO_BUILD_BUILD_DIR=$HOME/Library/Caches/$CONFIG_CLI/native-build
