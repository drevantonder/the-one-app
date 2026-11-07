#!/bin/sh
# Keeps what a Mac's update builds and throwaways leave behind in bounds, and says plainly when the Mac has no room to build.
# Run in a checkout's root (the update's, ~/Library/Caches/<cli>/src, or a throwaway's copy beside it).
#   disk.sh sweep            deletes the copies of a checkout a throwaway made here whose throwaway is gone
#   disk.sh prune            sweeps, clears this checkout's build caches when they're over their budget or the disk is low,
#                            then fails, saying what of ours is using the disk, if the disk is still low
#   disk.sh check            fails, saying how much is free and what of ours is using the disk, if an update has no room
#   disk.sh retry <command>  runs a build; if it fails on a damaged incremental cache, clears that cache and runs it once more
# Only this checkout's target/ and client/native/target/ (the app's own workspace) and dead throwaway copies are deleted:
# the builds own them, and rebuild them. Each target has its own budget.
set -u

# Where the update and the throwaways keep their checkouts: the folder this one is in.
case $PWD in
"$HOME"/Library/Caches/*/src | "$HOME"/Library/Caches/*/throwaway-*) cache=$(dirname "$PWD") ;;
*) cache= ;;
esac

# KB: what target/ may take before its caches go, and how much of the disk a build needs free.
BUDGET=${DISK_BUDGET_KB:-14680064}
NEEDED=${DISK_NEEDED_KB:-10485760}

kb() { du -sk "$1" 2>/dev/null | awk '{ s = $1 } END { print s + 0 }'; }
free() { df -k "$HOME" | awk 'NR == 2 { print $4 }'; }

# Incremental caches and the objects a killed or failed build left, which cargo never removes and which fill up the disk.
clear_incremental() {
	for t in target client/native/target; do
		rm -rf "$t"/*/incremental "$t"/*/*/incremental
		find "$t" -name '*.rcgu.o' -delete 2>/dev/null
	done
}

# A throwaway's copy is in use while its network's port, which its client's ssh forwards here, answers. The copy writes
# that port when it's made, so one made in the last few minutes (its forward not up yet) is left alone.
sweep() {
	[ -n "$cache" ] || return 0
	for d in "$cache"/throwaway-*; do
		[ -d "$d" ] || continue
		port=$(cat "$d/.throwaway-port" 2>/dev/null)
		if [ -n "$port" ] && nc -z 127.0.0.1 "$port" 2>/dev/null; then continue; fi
		ref=$d/.throwaway-port; [ -e "$ref" ] || ref=$d
		[ -z "$(find "$ref" -maxdepth 0 -mmin -10)" ] || continue
		echo "deleting $(basename "$d"), a throwaway's copy ($(($(kb "$d") / 1024)) MB) whose throwaway is gone"
		rm -rf "$d"
	done
}

# What of ours is using the disk, biggest first.
ours() {
	for d in "$cache"/* "$HOME/Library/Caches/Mozilla.sccache"; do
		[ -d "$d" ] && echo "$d"
	done | tr '\n' '\0' | xargs -0 du -sk 2>/dev/null | sort -rn | head -5 | awk '{ size = $1; sub(/^[0-9]+[ \t]+/, ""); printf "    %5.1f GB  %s\n", size / 1048576, $0 }'
}

# The biggest folders in the person's home, the guarded ones macOS asks about (Documents, Pictures…) left out.
biggest() {
	for d in "$HOME"/* "$HOME"/Library/* "$HOME"/Library/Caches/* "$HOME"/Library/Developer/*; do
		case $d in
		"$HOME"/Library | "$HOME"/Library/Caches | "$HOME"/Library/Developer | "$HOME"/Desktop | "$HOME"/Documents | "$HOME"/Downloads | "$HOME"/Pictures | "$HOME"/Movies | "$HOME"/Music) ;;
		*) [ -e "$d" ] && echo "$d" ;;
		esac
	done | tr '\n' '\0' | xargs -0 du -sk 2>/dev/null | sort -rn | head -8 | awk '{ size = $1; sub(/^[0-9]+[ \t]+/, ""); printf "    %5.1f GB  %s\n", size / 1048576, $0 }'
}

# Fails, saying why, when an update has no room.
check() {
	left=$(free)
	[ "$left" -lt "$NEEDED" ] || return 0
	echo "this Mac has only $((left / 1048576)) GB free, and an update needs $((NEEDED / 1048576)): it can't build here until something goes."
	[ -z "$cache" ] || { echo "Ours, using the disk:"; ours; }
	echo "The biggest folders:"
	biggest
	return 1
}

prune() {
	sweep
	for t in target client/native/target; do
		[ -d "$t" ] || mkdir -p "$t"
		if [ "$(kb "$t")" -gt "$BUDGET" ] || [ "$(free)" -lt "$NEEDED" ]; then
			echo "clearing the build's caches ($t/ is $(($(kb "$t") / 1048576)) GB, the disk has $(($(free) / 1048576)) GB free)"
			clear_incremental
			# Over budget still, or the disk: the debug builds, which the next one makes again.
			if [ "$(kb "$t")" -gt "$BUDGET" ] || [ "$(free)" -lt "$NEEDED" ]; then rm -rf "$t/debug"; fi
			if [ "$(kb "$t")" -gt "$BUDGET" ]; then rm -rf "$t"; mkdir -p "$t"; fi
		fi
	done
	check
}

case ${1:-} in
sweep) sweep ;;
prune) prune ;;
check) check ;;
retry)
	shift
	log=$(mktemp)
	"$@" > "$log" 2>&1
	status=$?
	cat "$log"
	if [ $status != 0 ] && grep -qE 'linking with|incremental|internal compiler error|No space left' "$log"; then
		rm -f "$log"
		echo "its build failed on its incremental cache, which looks damaged: clearing it and building once more"
		clear_incremental
		prune || exit 1
		"$@"
		exit
	fi
	rm -f "$log"
	exit $status
	;;
*) echo "usage: disk.sh sweep | prune | check | retry <command>" >&2; exit 2 ;;
esac
