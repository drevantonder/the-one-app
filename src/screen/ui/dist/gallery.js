import { c as e } from "react/compiler-runtime";
import * as t from "react";
import { createContext as n, useState as r } from "react";
import { createRoot as i } from "react-dom/client";
import { AlertDialog as a, AlertDialogAction as o, AlertDialogCancel as s, AlertDialogContent as c, AlertDialogDescription as l, AlertDialogFooter as u, AlertDialogHeader as d, AlertDialogTitle as f, AlertDialogTrigger as ee, AppIcon as p, Badge as m, Button as h, Card as te, CardContent as ne, CardDescription as re, CardFooter as ie, CardHeader as ae, CardTitle as oe, Dialog as se, DialogContent as ce, DialogDescription as le, DialogFooter as ue, DialogHeader as de, DialogTitle as fe, DialogTrigger as pe, DropdownMenu as me, DropdownMenuContent as he, DropdownMenuItem as ge, DropdownMenuLabel as _e, DropdownMenuSeparator as ve, DropdownMenuTrigger as ye, Empty as be, Grid as xe, Icon as g, Input as Se, List as Ce, ListRow as _, Problem as we, Separator as Te, Sheet as Ee, SheetContent as De, SheetDescription as Oe, SheetHeader as ke, SheetTitle as Ae, SheetTrigger as je, Sidebar as Me, SidebarContent as Ne, SidebarGroup as Pe, SidebarGroupLabel as Fe, SidebarMenu as Ie, SidebarMenuBadge as v, SidebarMenuButton as y, SidebarMenuItem as b, SidebarProvider as Le, Skeleton as Re, Split as x, Switch as S, Tabs as ze, TabsContent as Be, TabsList as Ve, TabsTrigger as He } from "@one/ui";
import { jsx as C, jsxs as w } from "react/jsx-runtime";
// node_modules/@phosphor-icons/react/dist/defs/DotsThree.es.js
var T = /* @__PURE__ */ new Map([
	["bold", /* @__PURE__ */ t.createElement(t.Fragment, null, /* @__PURE__ */ t.createElement("path", { d: "M144,128a16,16,0,1,1-16-16A16,16,0,0,1,144,128ZM60,112a16,16,0,1,0,16,16A16,16,0,0,0,60,112Zm136,0a16,16,0,1,0,16,16A16,16,0,0,0,196,112Z" }))],
	["duotone", /* @__PURE__ */ t.createElement(t.Fragment, null, /* @__PURE__ */ t.createElement("path", {
		d: "M240,96v64a16,16,0,0,1-16,16H32a16,16,0,0,1-16-16V96A16,16,0,0,1,32,80H224A16,16,0,0,1,240,96Z",
		opacity: "0.2"
	}), /* @__PURE__ */ t.createElement("path", { d: "M140,128a12,12,0,1,1-12-12A12,12,0,0,1,140,128Zm56-12a12,12,0,1,0,12,12A12,12,0,0,0,196,116ZM60,116a12,12,0,1,0,12,12A12,12,0,0,0,60,116Z" }))],
	["fill", /* @__PURE__ */ t.createElement(t.Fragment, null, /* @__PURE__ */ t.createElement("path", { d: "M224,80H32A16,16,0,0,0,16,96v64a16,16,0,0,0,16,16H224a16,16,0,0,0,16-16V96A16,16,0,0,0,224,80ZM60,140a12,12,0,1,1,12-12A12,12,0,0,1,60,140Zm68,0a12,12,0,1,1,12-12A12,12,0,0,1,128,140Zm68,0a12,12,0,1,1,12-12A12,12,0,0,1,196,140Z" }))],
	["light", /* @__PURE__ */ t.createElement(t.Fragment, null, /* @__PURE__ */ t.createElement("path", { d: "M138,128a10,10,0,1,1-10-10A10,10,0,0,1,138,128ZM60,118a10,10,0,1,0,10,10A10,10,0,0,0,60,118Zm136,0a10,10,0,1,0,10,10A10,10,0,0,0,196,118Z" }))],
	["regular", /* @__PURE__ */ t.createElement(t.Fragment, null, /* @__PURE__ */ t.createElement("path", { d: "M140,128a12,12,0,1,1-12-12A12,12,0,0,1,140,128Zm56-12a12,12,0,1,0,12,12A12,12,0,0,0,196,116ZM60,116a12,12,0,1,0,12,12A12,12,0,0,0,60,116Z" }))],
	["thin", /* @__PURE__ */ t.createElement(t.Fragment, null, /* @__PURE__ */ t.createElement("path", { d: "M136,128a8,8,0,1,1-8-8A8,8,0,0,1,136,128Zm-76-8a8,8,0,1,0,8,8A8,8,0,0,0,60,120Zm136,0a8,8,0,1,0,8,8A8,8,0,0,0,196,120Z" }))]
]), E = /* @__PURE__ */ new Map([
	["bold", /* @__PURE__ */ t.createElement(t.Fragment, null, /* @__PURE__ */ t.createElement("path", { d: "M232.49,215.51,185,168a92.12,92.12,0,1,0-17,17l47.53,47.54a12,12,0,0,0,17-17ZM44,112a68,68,0,1,1,68,68A68.07,68.07,0,0,1,44,112Z" }))],
	["duotone", /* @__PURE__ */ t.createElement(t.Fragment, null, /* @__PURE__ */ t.createElement("path", {
		d: "M192,112a80,80,0,1,1-80-80A80,80,0,0,1,192,112Z",
		opacity: "0.2"
	}), /* @__PURE__ */ t.createElement("path", { d: "M229.66,218.34,179.6,168.28a88.21,88.21,0,1,0-11.32,11.31l50.06,50.07a8,8,0,0,0,11.32-11.32ZM40,112a72,72,0,1,1,72,72A72.08,72.08,0,0,1,40,112Z" }))],
	["fill", /* @__PURE__ */ t.createElement(t.Fragment, null, /* @__PURE__ */ t.createElement("path", { d: "M168,112a56,56,0,1,1-56-56A56,56,0,0,1,168,112Zm61.66,117.66a8,8,0,0,1-11.32,0l-50.06-50.07a88,88,0,1,1,11.32-11.31l50.06,50.06A8,8,0,0,1,229.66,229.66ZM112,184a72,72,0,1,0-72-72A72.08,72.08,0,0,0,112,184Z" }))],
	["light", /* @__PURE__ */ t.createElement(t.Fragment, null, /* @__PURE__ */ t.createElement("path", { d: "M228.24,219.76l-51.38-51.38a86.15,86.15,0,1,0-8.48,8.48l51.38,51.38a6,6,0,0,0,8.48-8.48ZM38,112a74,74,0,1,1,74,74A74.09,74.09,0,0,1,38,112Z" }))],
	["regular", /* @__PURE__ */ t.createElement(t.Fragment, null, /* @__PURE__ */ t.createElement("path", { d: "M229.66,218.34l-50.07-50.06a88.11,88.11,0,1,0-11.31,11.31l50.06,50.07a8,8,0,0,0,11.32-11.32ZM40,112a72,72,0,1,1,72,72A72.08,72.08,0,0,1,40,112Z" }))],
	["thin", /* @__PURE__ */ t.createElement(t.Fragment, null, /* @__PURE__ */ t.createElement("path", { d: "M226.83,221.17l-52.7-52.7a84.1,84.1,0,1,0-5.66,5.66l52.7,52.7a4,4,0,0,0,5.66-5.66ZM36,112a76,76,0,1,1,76,76A76.08,76.08,0,0,1,36,112Z" }))]
]), D = /* @__PURE__ */ new Map([
	["bold", /* @__PURE__ */ t.createElement(t.Fragment, null, /* @__PURE__ */ t.createElement("path", { d: "M228,128a12,12,0,0,1-12,12H140v76a12,12,0,0,1-24,0V140H40a12,12,0,0,1,0-24h76V40a12,12,0,0,1,24,0v76h76A12,12,0,0,1,228,128Z" }))],
	["duotone", /* @__PURE__ */ t.createElement(t.Fragment, null, /* @__PURE__ */ t.createElement("path", {
		d: "M216,56V200a16,16,0,0,1-16,16H56a16,16,0,0,1-16-16V56A16,16,0,0,1,56,40H200A16,16,0,0,1,216,56Z",
		opacity: "0.2"
	}), /* @__PURE__ */ t.createElement("path", { d: "M224,128a8,8,0,0,1-8,8H136v80a8,8,0,0,1-16,0V136H40a8,8,0,0,1,0-16h80V40a8,8,0,0,1,16,0v80h80A8,8,0,0,1,224,128Z" }))],
	["fill", /* @__PURE__ */ t.createElement(t.Fragment, null, /* @__PURE__ */ t.createElement("path", { d: "M208,32H48A16,16,0,0,0,32,48V208a16,16,0,0,0,16,16H208a16,16,0,0,0,16-16V48A16,16,0,0,0,208,32ZM184,136H136v48a8,8,0,0,1-16,0V136H72a8,8,0,0,1,0-16h48V72a8,8,0,0,1,16,0v48h48a8,8,0,0,1,0,16Z" }))],
	["light", /* @__PURE__ */ t.createElement(t.Fragment, null, /* @__PURE__ */ t.createElement("path", { d: "M222,128a6,6,0,0,1-6,6H134v82a6,6,0,0,1-12,0V134H40a6,6,0,0,1,0-12h82V40a6,6,0,0,1,12,0v82h82A6,6,0,0,1,222,128Z" }))],
	["regular", /* @__PURE__ */ t.createElement(t.Fragment, null, /* @__PURE__ */ t.createElement("path", { d: "M224,128a8,8,0,0,1-8,8H136v80a8,8,0,0,1-16,0V136H40a8,8,0,0,1,0-16h80V40a8,8,0,0,1,16,0v80h80A8,8,0,0,1,224,128Z" }))],
	["thin", /* @__PURE__ */ t.createElement(t.Fragment, null, /* @__PURE__ */ t.createElement("path", { d: "M220,128a4,4,0,0,1-4,4H132v84a4,4,0,0,1-8,0V132H40a4,4,0,0,1,0-8h84V40a4,4,0,0,1,8,0v84h84A4,4,0,0,1,220,128Z" }))]
]), O = n({
	color: "currentColor",
	size: "1em",
	weight: "regular",
	mirrored: !1
}), k = t.forwardRef((e, n) => {
	let { alt: r, color: i, size: a, weight: o, mirrored: s, children: c, weights: l, ...u } = e, { color: d = "currentColor", size: f, weight: ee = "regular", mirrored: p = !1, ...m } = t.useContext(O);
	return /* @__PURE__ */ t.createElement("svg", {
		ref: n,
		xmlns: "http://www.w3.org/2000/svg",
		width: a ?? f,
		height: a ?? f,
		fill: i ?? d,
		viewBox: "0 0 256 256",
		transform: s || p ? "scale(-1, 1)" : void 0,
		...m,
		...u
	}, !!r && /* @__PURE__ */ t.createElement("title", null, r), c, l.get(o ?? ee));
});
k.displayName = "IconBase";
// node_modules/@phosphor-icons/react/dist/csr/DotsThree.es.js
var Ue = t.forwardRef((e, n) => /* @__PURE__ */ t.createElement(k, {
	ref: n,
	...e,
	weights: T
}));
Ue.displayName = "DotsThreeIcon";
// node_modules/@phosphor-icons/react/dist/csr/MagnifyingGlass.es.js
var We = t.forwardRef((e, n) => /* @__PURE__ */ t.createElement(k, {
	ref: n,
	...e,
	weights: E
}));
We.displayName = "MagnifyingGlassIcon";
// node_modules/@phosphor-icons/react/dist/csr/Plus.es.js
var Ge = t.forwardRef((e, n) => /* @__PURE__ */ t.createElement(k, {
	ref: n,
	...e,
	weights: D
}));
Ge.displayName = "PlusIcon";
// gallery/gallery.tsx
var A = [
	{
		name: "agent-devices",
		title: "Agent Devices",
		icon: "browsers",
		color: "#a2845e"
	},
	{
		name: "board",
		title: "Agent Board",
		icon: "chat-text",
		color: "#ff9500"
	},
	{
		name: "device-activity",
		title: "Device Activity",
		icon: "monitor",
		color: "#30b0c7"
	},
	{
		name: "repos",
		title: "Repos",
		icon: "git-branch",
		color: "#5856d6"
	},
	{
		name: "computer",
		title: "Computer Use",
		icon: "cursor-click",
		color: "#5e5ce6"
	},
	{
		name: "groups",
		title: "Groups",
		icon: "users-three",
		color: "#34c759"
	},
	{
		name: "connect-ai",
		title: "Connect Your AI",
		icon: "plugs-connected",
		color: "#ff2d55"
	},
	{
		name: "backups",
		title: "Backups",
		icon: "hard-drives",
		color: "#ff9500"
	},
	{
		name: "labs",
		title: "Labs",
		icon: "flask",
		color: "#af52de"
	},
	{
		name: "devices",
		title: "Devices",
		icon: "devices",
		color: "#8e8e93"
	}
], j = [
	{
		id: "anthropic",
		title: "Anthropic",
		sub: "Key · Used by AI and Models",
		app: {
			name: "ai",
			icon: "sparkle",
			color: "#d97757"
		}
	},
	{
		id: "github",
		title: "GitHub",
		sub: "Account · Used by Repos",
		app: {
			name: "github",
			icon: "github-logo",
			color: "#24292f"
		}
	},
	{
		id: "work",
		title: "andre@work",
		sub: "Google account · Used by Gmail and Drive",
		app: {
			name: "google",
			icon: "google-logo",
			color: "#4285f4"
		}
	},
	{
		id: "wifi",
		title: "Office Wi-Fi",
		sub: "network: Office-5G",
		app: {
			name: "wifi",
			icon: "wifi-high",
			color: "#8e8e93"
		}
	}
];
function M(t) {
	let n = e(5), { title: r, children: i } = t, a;
	n[0] === r ? a = n[1] : (a = /* @__PURE__ */ C("h2", {
		className: "text-sm font-medium text-muted-foreground",
		children: r
	}), n[0] = r, n[1] = a);
	let o;
	return n[2] !== i || n[3] !== a ? (o = /* @__PURE__ */ w("section", {
		className: "flex flex-col gap-3",
		children: [a, i]
	}), n[2] = i, n[3] = a, n[4] = o) : o = n[4], o;
}
function N(t) {
	let n = e(2), { children: r } = t, i;
	return n[0] === r ? i = n[1] : (i = /* @__PURE__ */ C("div", {
		className: "flex flex-wrap items-center gap-2",
		children: r
	}), n[0] = r, n[1] = i), i;
}
function Ke() {
	let t = e(13), [n, i] = r(null), a;
	t[0] === n ? a = t[1] : (a = j.find((e) => e.id === n), t[0] = n, t[1] = a);
	let o = a, s = o?.title, c;
	t[2] === Symbol.for("react.memo_cache_sentinel") ? (c = () => i(null), t[2] = c) : c = t[2];
	let l;
	t[3] === n ? l = t[4] : (l = j.map((e) => /* @__PURE__ */ C(_, {
		leading: /* @__PURE__ */ C(p, {
			app: e.app,
			size: 32
		}),
		title: e.title,
		sub: e.sub,
		selected: e.id === n,
		onClick: () => i(e.id)
	}, e.id)), t[3] = n, t[4] = l);
	let u;
	t[5] === l ? u = t[6] : (u = /* @__PURE__ */ C(Ce, { children: l }), t[5] = l, t[6] = u);
	let d;
	t[7] === o ? d = t[8] : (d = o && /* @__PURE__ */ w(te, { children: [
		/* @__PURE__ */ w(ae, {
			className: "flex flex-row items-center gap-4",
			children: [
				/* @__PURE__ */ C(p, {
					app: o.app,
					size: 56
				}),
				/* @__PURE__ */ w("div", {
					className: "min-w-0 flex-1",
					children: [/* @__PURE__ */ C(oe, {
						className: "text-lg",
						children: o.title
					}), /* @__PURE__ */ C(re, { children: "You" })]
				}),
				/* @__PURE__ */ C(h, {
					variant: "outline",
					size: "sm",
					children: "Rename"
				})
			]
		}),
		/* @__PURE__ */ w(ne, {
			className: "flex flex-col gap-3",
			children: [/* @__PURE__ */ w("div", {
				className: "flex items-center justify-between gap-3",
				children: [/* @__PURE__ */ C("span", {
					className: "text-sm",
					children: "AI"
				}), /* @__PURE__ */ C(S, { defaultChecked: !0 })]
			}), /* @__PURE__ */ w("div", {
				className: "flex items-center justify-between gap-3",
				children: [/* @__PURE__ */ C("span", {
					className: "text-sm",
					children: "AGENTS.md"
				}), /* @__PURE__ */ C(S, {})]
			})]
		}),
		/* @__PURE__ */ C(ie, { children: /* @__PURE__ */ C(h, {
			variant: "destructive",
			size: "sm",
			children: "Delete Item"
		}) })
	] }), t[7] = o, t[8] = d);
	let f;
	return t[9] !== s || t[10] !== u || t[11] !== d ? (f = /* @__PURE__ */ C(x, {
		title: s,
		onClose: c,
		list: u,
		detail: d
	}), t[9] = s, t[10] = u, t[11] = d, t[12] = f) : f = t[12], f;
}
function P() {
	let t = e(37), n;
	t[0] === Symbol.for("react.memo_cache_sentinel") ? (n = /* @__PURE__ */ C("h1", {
		className: "text-2xl font-semibold tracking-tight",
		children: "UI kit"
	}), t[0] = n) : n = t[0];
	let r;
	t[1] === Symbol.for("react.memo_cache_sentinel") ? (r = /* @__PURE__ */ w(N, { children: [
		/* @__PURE__ */ C(h, { children: "Default" }),
		/* @__PURE__ */ C(h, {
			variant: "secondary",
			children: "Secondary"
		}),
		/* @__PURE__ */ C(h, {
			variant: "outline",
			children: "Outline"
		}),
		/* @__PURE__ */ C(h, {
			variant: "ghost",
			children: "Ghost"
		}),
		/* @__PURE__ */ C(h, {
			variant: "destructive",
			children: "Destructive"
		}),
		/* @__PURE__ */ C(h, {
			variant: "link",
			children: "Link"
		})
	] }), t[1] = r) : r = t[1];
	let i, g;
	t[2] === Symbol.for("react.memo_cache_sentinel") ? (i = /* @__PURE__ */ w(h, {
		size: "sm",
		variant: "outline",
		children: [/* @__PURE__ */ C(Ge, {}), " New Item"]
	}), g = /* @__PURE__ */ C(h, {
		size: "lg",
		children: "Large"
	}), t[2] = i, t[3] = g) : (i = t[2], g = t[3]);
	let _;
	t[4] === Symbol.for("react.memo_cache_sentinel") ? (_ = /* @__PURE__ */ w(M, {
		title: "Button: one default per view, then secondary, outline, ghost; destructive only for what can't be undone",
		children: [r, /* @__PURE__ */ w(N, { children: [
			i,
			g,
			/* @__PURE__ */ C(h, {
				size: "icon",
				variant: "ghost",
				"aria-label": "More",
				children: /* @__PURE__ */ C(Ue, { weight: "bold" })
			}),
			/* @__PURE__ */ C(h, {
				disabled: !0,
				children: "Disabled"
			})
		] })]
	}), t[4] = _) : _ = t[4];
	let v;
	t[5] === Symbol.for("react.memo_cache_sentinel") ? (v = /* @__PURE__ */ C(M, {
		title: "Badge",
		children: /* @__PURE__ */ w(N, { children: [
			/* @__PURE__ */ C(m, { children: "Default" }),
			/* @__PURE__ */ C(m, {
				variant: "secondary",
				children: "Secondary"
			}),
			/* @__PURE__ */ C(m, {
				variant: "outline",
				children: "Outline"
			}),
			/* @__PURE__ */ C(m, {
				variant: "destructive",
				className: "rounded-full",
				children: "3"
			})
		] })
	}), t[5] = v) : v = t[5];
	let y;
	t[6] === Symbol.for("react.memo_cache_sentinel") ? (y = /* @__PURE__ */ C(M, {
		title: "Input",
		children: /* @__PURE__ */ w("div", {
			className: "relative max-w-sm",
			children: [/* @__PURE__ */ C(We, { className: "pointer-events-none absolute top-1/2 left-2.5 size-4 -translate-y-1/2 text-muted-foreground" }), /* @__PURE__ */ C(Se, {
				placeholder: "Search",
				className: "pl-8"
			})]
		})
	}), t[6] = y) : y = t[6];
	let b;
	t[7] === Symbol.for("react.memo_cache_sentinel") ? (b = /* @__PURE__ */ C(M, {
		title: "Tabs",
		children: /* @__PURE__ */ w(ze, {
			defaultValue: "you",
			className: "max-w-md",
			children: [
				/* @__PURE__ */ w(Ve, { children: [
					/* @__PURE__ */ C(He, {
						value: "you",
						children: "You"
					}),
					/* @__PURE__ */ C(He, {
						value: "team",
						children: "Team"
					}),
					/* @__PURE__ */ C(He, {
						value: "everyone",
						children: "Everyone"
					})
				] }),
				/* @__PURE__ */ C(Be, {
					value: "you",
					className: "text-sm text-muted-foreground",
					children: "Yours alone."
				}),
				/* @__PURE__ */ C(Be, {
					value: "team",
					className: "text-sm text-muted-foreground",
					children: "Your team's."
				}),
				/* @__PURE__ */ C(Be, {
					value: "everyone",
					className: "text-sm text-muted-foreground",
					children: "Everyone's."
				})
			]
		})
	}), t[7] = b) : b = t[7];
	let x;
	t[8] === Symbol.for("react.memo_cache_sentinel") ? (x = /* @__PURE__ */ C(M, {
		title: "Switch",
		children: /* @__PURE__ */ w(N, { children: [
			/* @__PURE__ */ C(S, {
				defaultChecked: !0,
				"aria-label": "On"
			}),
			/* @__PURE__ */ C(S, { "aria-label": "Off" }),
			/* @__PURE__ */ C(S, {
				disabled: !0,
				"aria-label": "Disabled"
			})
		] })
	}), t[8] = x) : x = t[8];
	let T, E;
	t[9] === Symbol.for("react.memo_cache_sentinel") ? (E = /* @__PURE__ */ w(ae, { children: [/* @__PURE__ */ C(oe, { children: "Backups" }), /* @__PURE__ */ C(re, { children: "Encrypted copies of everything." })] }), T = /* @__PURE__ */ C(ne, {
		className: "text-sm",
		children: "Last backed up 5 minutes ago."
	}), t[9] = T, t[10] = E) : (T = t[9], E = t[10]);
	let D;
	t[11] === Symbol.for("react.memo_cache_sentinel") ? (D = /* @__PURE__ */ C(M, {
		title: "Card",
		children: /* @__PURE__ */ w(te, {
			className: "max-w-sm",
			children: [
				E,
				T,
				/* @__PURE__ */ C(ie, { children: /* @__PURE__ */ C(h, {
					variant: "outline",
					size: "sm",
					children: "Back Up Now"
				}) })
			]
		})
	}), t[11] = D) : D = t[11];
	let O;
	t[12] === Symbol.for("react.memo_cache_sentinel") ? (O = /* @__PURE__ */ C(ye, {
		asChild: !0,
		children: /* @__PURE__ */ C(h, {
			variant: "outline",
			children: "Menu"
		})
	}), t[12] = O) : O = t[12];
	let k;
	t[13] === Symbol.for("react.memo_cache_sentinel") ? (k = /* @__PURE__ */ w(me, { children: [O, /* @__PURE__ */ w(he, { children: [
		/* @__PURE__ */ C(_e, { children: "Agent Devices" }),
		/* @__PURE__ */ C(ve, {}),
		/* @__PURE__ */ C(ge, { children: "Open" }),
		/* @__PURE__ */ C(ge, { children: "Rename" }),
		/* @__PURE__ */ C(ge, {
			variant: "destructive",
			children: "Delete"
		})
	] })] }), t[13] = k) : k = t[13];
	let j;
	t[14] === Symbol.for("react.memo_cache_sentinel") ? (j = /* @__PURE__ */ C(pe, {
		asChild: !0,
		children: /* @__PURE__ */ C(h, {
			variant: "outline",
			children: "Dialog"
		})
	}), t[14] = j) : j = t[14];
	let P, F;
	t[15] === Symbol.for("react.memo_cache_sentinel") ? (P = /* @__PURE__ */ w(de, { children: [/* @__PURE__ */ C(fe, { children: "Rename" }), /* @__PURE__ */ C(le, { children: "What it's called on every device." })] }), F = /* @__PURE__ */ C(Se, { defaultValue: "Anthropic" }), t[15] = P, t[16] = F) : (P = t[15], F = t[16]);
	let I;
	t[17] === Symbol.for("react.memo_cache_sentinel") ? (I = /* @__PURE__ */ w(se, { children: [j, /* @__PURE__ */ w(ce, { children: [
		P,
		F,
		/* @__PURE__ */ C(ue, { children: /* @__PURE__ */ C(h, { children: "Save" }) })
	] })] }), t[17] = I) : I = t[17];
	let L;
	t[18] === Symbol.for("react.memo_cache_sentinel") ? (L = /* @__PURE__ */ C(ee, {
		asChild: !0,
		children: /* @__PURE__ */ C(h, {
			variant: "outline",
			children: "Alert dialog"
		})
	}), t[18] = L) : L = t[18];
	let R;
	t[19] === Symbol.for("react.memo_cache_sentinel") ? (R = /* @__PURE__ */ w(d, { children: [/* @__PURE__ */ C(f, { children: "Delete Anthropic?" }), /* @__PURE__ */ C(l, { children: "Apps using it stop working until you add another." })] }), t[19] = R) : R = t[19];
	let z;
	t[20] === Symbol.for("react.memo_cache_sentinel") ? (z = /* @__PURE__ */ w(a, { children: [L, /* @__PURE__ */ w(c, { children: [R, /* @__PURE__ */ w(u, { children: [/* @__PURE__ */ C(s, { children: "Cancel" }), /* @__PURE__ */ C(o, {
		className: "bg-destructive text-white hover:bg-destructive/90",
		children: "Delete"
	})] })] })] }), t[20] = z) : z = t[20];
	let B;
	t[21] === Symbol.for("react.memo_cache_sentinel") ? (B = /* @__PURE__ */ C(je, {
		asChild: !0,
		children: /* @__PURE__ */ C(h, {
			variant: "outline",
			children: "Sheet"
		})
	}), t[21] = B) : B = t[21];
	let V;
	t[22] === Symbol.for("react.memo_cache_sentinel") ? (V = /* @__PURE__ */ C(M, {
		title: "Dropdown menu, dialog, alert dialog, sheet",
		children: /* @__PURE__ */ w(N, { children: [
			k,
			I,
			z,
			/* @__PURE__ */ w(Ee, { children: [B, /* @__PURE__ */ C(De, { children: /* @__PURE__ */ w(ke, { children: [/* @__PURE__ */ C(Ae, { children: "Anthropic" }), /* @__PURE__ */ C(Oe, { children: "Key · Used by AI and Models" })] }) })] })
		] })
	}), t[22] = V) : V = t[22];
	let H;
	t[23] === Symbol.for("react.memo_cache_sentinel") ? (H = /* @__PURE__ */ C(M, {
		title: "Separator and skeleton",
		children: /* @__PURE__ */ w("div", {
			className: "flex max-w-sm flex-col gap-3",
			children: [
				/* @__PURE__ */ C(Re, { className: "h-4 w-3/4" }),
				/* @__PURE__ */ C(Re, { className: "h-4 w-1/2" }),
				/* @__PURE__ */ C(Te, {}),
				/* @__PURE__ */ C(Re, { className: "aspect-[16/10] w-full rounded-xl" })
			]
		})
	}), t[23] = H) : H = t[23];
	let U;
	t[24] === Symbol.for("react.memo_cache_sentinel") ? (U = /* @__PURE__ */ C(we, {
		title: "Couldn't reach your devices.",
		text: "Check your connection.",
		action: {
			label: "Try Again",
			run: et
		}
	}), t[24] = U) : U = t[24];
	let W;
	t[25] === Symbol.for("react.memo_cache_sentinel") ? (W = /* @__PURE__ */ w(M, {
		title: "Problem: on the thing it's about, with the action that fixes it",
		children: [U, /* @__PURE__ */ C(we, {
			warn: !0,
			title: "None of your devices runs Agent Devices.",
			text: "It needs a Mac, or a Linux device with Podman.",
			action: {
				label: "Open App Store",
				run: $e
			}
		})]
	}), t[25] = W) : W = t[25];
	let G;
	t[26] === Symbol.for("react.memo_cache_sentinel") ? (G = /* @__PURE__ */ C(M, {
		title: "Empty: one grey line",
		children: /* @__PURE__ */ C(te, { children: /* @__PURE__ */ C(be, { children: "None yet. Agents make one when they need a browser." }) })
	}), t[26] = G) : G = t[26];
	let K;
	t[27] === Symbol.for("react.memo_cache_sentinel") ? (K = A.map(Qe), t[27] = K) : K = t[27];
	let q;
	t[28] === Symbol.for("react.memo_cache_sentinel") ? (q = /* @__PURE__ */ w(N, { children: [K, /* @__PURE__ */ C(p, { app: {
		name: "x",
		title: "Xylophone"
	} })] }), t[28] = q) : q = t[28];
	let J;
	t[29] === Symbol.for("react.memo_cache_sentinel") ? (J = /* @__PURE__ */ w(M, {
		title: "App icons, in each app's colour, and icons by name",
		children: [q, /* @__PURE__ */ C(N, { children: [
			"magnifying-glass",
			"gear",
			"warning-circle-fill",
			"check-circle-fill",
			"browsers"
		].map(Ze) })]
	}), t[29] = J) : J = t[29];
	let Y;
	t[30] === Symbol.for("react.memo_cache_sentinel") ? (Y = /* @__PURE__ */ C(M, {
		title: "Grid: fills the room, at most 4 across here",
		children: /* @__PURE__ */ C(xe, {
			min: "12rem",
			max: 4,
			children: A.slice(0, 6).map(Xe)
		})
	}), t[30] = Y) : Y = t[30];
	let X;
	t[31] === Symbol.for("react.memo_cache_sentinel") ? (X = /* @__PURE__ */ C(M, {
		title: "List: rows that stop at a readable width",
		children: /* @__PURE__ */ C(Ce, { children: A.slice(0, 4).map(Ye) })
	}), t[31] = X) : X = t[31];
	let Z;
	t[32] === Symbol.for("react.memo_cache_sentinel") ? (Z = /* @__PURE__ */ C(M, {
		title: "Split: what a list opens shows beside it where there's room, else in a sheet",
		children: /* @__PURE__ */ C(Ke, {})
	}), t[32] = Z) : Z = t[32];
	let Q;
	t[33] === Symbol.for("react.memo_cache_sentinel") ? (Q = /* @__PURE__ */ C(Fe, { children: "You" }), t[33] = Q) : Q = t[33];
	let tt;
	t[34] === Symbol.for("react.memo_cache_sentinel") ? (tt = /* @__PURE__ */ w(Pe, { children: [Q, /* @__PURE__ */ C(Ie, { children: A.slice(0, 2).map(Je) })] }), t[34] = tt) : tt = t[34];
	let $;
	t[35] === Symbol.for("react.memo_cache_sentinel") ? ($ = /* @__PURE__ */ C(Fe, { children: "Settings" }), t[35] = $) : $ = t[35];
	let nt;
	return t[36] === Symbol.for("react.memo_cache_sentinel") ? (nt = /* @__PURE__ */ w("main", {
		className: "mx-auto flex flex-col gap-10",
		children: [
			n,
			_,
			v,
			y,
			b,
			x,
			D,
			V,
			H,
			W,
			G,
			J,
			Y,
			X,
			Z,
			/* @__PURE__ */ C(M, {
				title: "Sidebar pieces",
				children: /* @__PURE__ */ C("div", {
					className: "h-[28rem] w-64 overflow-hidden rounded-xl border",
					children: /* @__PURE__ */ C(Le, {
						className: "min-h-0",
						children: /* @__PURE__ */ C(Me, {
							collapsible: "none",
							className: "w-full",
							children: /* @__PURE__ */ w(Ne, { children: [tt, /* @__PURE__ */ w(Pe, { children: [$, /* @__PURE__ */ C(Ie, { children: A.slice(5).map(qe) })] })] })
						})
					})
				})
			})
		]
	}), t[36] = nt) : nt = t[36], nt;
}
function qe(e) {
	return /* @__PURE__ */ w(b, { children: [/* @__PURE__ */ w(y, { children: [/* @__PURE__ */ C(p, {
		app: e,
		size: 20
	}), e.title] }), e.name === "devices" && /* @__PURE__ */ C(v, {
		className: "rounded-full bg-destructive text-white",
		children: "1"
	})] }, e.name);
}
function Je(e, t) {
	return /* @__PURE__ */ C(b, { children: /* @__PURE__ */ w(y, {
		isActive: t === 0,
		children: [/* @__PURE__ */ C(p, {
			app: e,
			size: 20
		}), e.title]
	}) }, e.name);
}
function Ye(e) {
	return /* @__PURE__ */ C(_, {
		leading: /* @__PURE__ */ C(p, {
			app: e,
			size: 28
		}),
		title: e.title,
		sub: "On every device",
		trailing: e.name === "devices" ? /* @__PURE__ */ C(m, {
			variant: "destructive",
			className: "rounded-full",
			children: "1"
		}) : void 0
	}, e.name);
}
function Xe(e) {
	return /* @__PURE__ */ w("div", {
		className: "min-w-0",
		children: [
			/* @__PURE__ */ C("div", {
				className: "grid aspect-[16/10] place-items-center rounded-xl bg-muted",
				children: /* @__PURE__ */ C(p, {
					app: e,
					size: 48
				})
			}),
			/* @__PURE__ */ C("div", {
				className: "mt-2 truncate text-sm font-medium",
				children: e.title
			}),
			/* @__PURE__ */ C("div", {
				className: "truncate text-sm text-muted-foreground",
				children: "personal"
			})
		]
	}, e.name);
}
function Ze(e) {
	return /* @__PURE__ */ C(g, {
		name: e,
		className: "size-5"
	}, e);
}
function Qe(e) {
	return /* @__PURE__ */ C(p, { app: e }, e.name);
}
function $e() {}
function et() {}
i(document.getElementById("root")).render(/* @__PURE__ */ C(P, {}));
