#!/bin/sh
# Keeps what a Mac's update builds leave behind in bounds, and says plainly when the Mac has no room to build.
#   disk.sh prune            clears this checkout's build caches when they're over their budget or the disk is low,
#                            then fails, naming the biggest folders, if the disk is still low
#   disk.sh retry <command>  runs a build; if it fails on a damaged incremental cache, clears that cache and runs it once more
# Only target/ in this checkout is touched: the build owns it, and rebuilds it.
set -u
cd "$(dirname "$0")/.."

# KB: what target/ may take before its caches go, and how much of the disk a build needs free.
BUDGET=${DISK_BUDGET_KB:-14680064}
NEEDED=${DISK_NEEDED_KB:-10485760}

kb() { du -sk "$1" 2>/dev/null | cut -f1; }
free() { df -k "$HOME" | awk 'NR == 2 { print $4 }'; }

# Incremental caches and the objects a killed or failed build left, which cargo never removes and which fill up the disk.
clear_incremental() {
	rm -rf target/*/incremental target/*/*/incremental
	find target -name '*.rcgu.o' -delete 2>/dev/null
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

prune() {
	[ -d target ] || mkdir -p target
	if [ "$(kb target)" -gt "$BUDGET" ] || [ "$(free)" -lt "$NEEDED" ]; then
		echo "clearing the build's caches (target/ is $(($(kb target) / 1048576)) GB, the disk has $(($(free) / 1048576)) GB free)"
		clear_incremental
		# Over budget still, or the disk: the bindings' own build, which the next one makes again.
		if [ "$(kb target)" -gt "$BUDGET" ] || [ "$(free)" -lt "$NEEDED" ]; then rm -rf target/debug; fi
		if [ "$(kb target)" -gt "$BUDGET" ]; then rm -rf target; mkdir -p target; fi
	fi
	left=$(free)
	if [ "$left" -lt "$NEEDED" ]; then
		echo "this Mac has only $((left / 1048576)) GB free, and a build needs $((NEEDED / 1048576)): it can't build here until something goes. The biggest folders:"
		biggest
		return 1
	fi
}

case ${1:-} in
prune) prune ;;
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
*) echo "usage: disk.sh prune | retry <command>" >&2; exit 2 ;;
esac
