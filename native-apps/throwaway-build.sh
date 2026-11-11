# Sourced by the Mac's and iPhone's throwaway.sh, with their arguments: every checkout's copy here builds in one folder,
# one build at a time, which takes only the files whose contents changed, so cargo and Xcode rebuild only what did (they
# go by a file's time, and a copy's are all new). disk.sh's sweep deletes it when the disk runs low.
alone "$@"
# Spotlight leaves a folder named .noindex alone, and would otherwise index every build output in it.
shared=$HOME/Library/Caches/$CONFIG_CLI/throwaway.noindex
if [ "$root" != "$shared" ]; then
	mkdir -p "$shared"
	# What the last copy put there and this one hasn't goes, leaving what its builds made.
	(cd "$root" && find . -type f -o -type l | sort) >"$shared.files"
	[ ! -f "$shared/.files" ] || comm -23 "$shared/.files" "$shared.files" | (cd "$shared" && tr '\n' '\0' | xargs -0 rm -f)
	rsync -rlc "$root/" "$shared/"
	mv "$shared.files" "$shared/.files"
	self=$(cd "$(dirname "$0")" && pwd)/$(basename "$0")
	exec "$shared/${self#"$root"/}" "$@"
fi
