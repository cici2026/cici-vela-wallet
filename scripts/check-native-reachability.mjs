#!/usr/bin/env node
/**
 * Every screen family a native client ships must be reachable from that
 * client's navigation root.
 *
 * This exists because spec 022 shipped 9,695 lines of drawn, translated,
 * fixture-complete UI on three clients that no person could open, and every
 * gate in the repository stayed green throughout. The specific shapes:
 *
 *   - desktop  `src/explore/` and `src/signing/` exist on disk and are not
 *              declared in `main.rs`, so rustc never reads them. Rust does not
 *              warn about a directory nobody declared.
 *   - Android  `feature/explore/` and `feature/signing/` compile (the package
 *              is on the source path) but no `VelaDestinations` route and no
 *              `composable(...)` reaches them.
 *   - iOS      `Features/Explore/` and `Features/Signing/` compile (folder-
 *              synced target) but `PageOverride.Page` has no case and
 *              `ExploreScreen(` is never instantiated.
 *
 * Note what those three have in common: on every platform the compiler was
 * happy. Only navigation can see this, which is why the check is navigation-
 * shaped rather than a lint.
 *
 * Deliberately static — no cargo, no gradle, no xcodebuild. It runs in the
 * `app` job, which already exists, in well under a second. A guard that needs
 * a 25-minute toolchain is a guard somebody eventually skips.
 *
 * Usage: node scripts/check-native-reachability.mjs
 * Exit 0 = every screen family is reachable. Exit 1 = names the orphans.
 */

import { readFileSync, readdirSync, existsSync } from 'node:fs';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';

const ROOT = join(dirname(fileURLToPath(import.meta.url)), '..');
const read = (p) => readFileSync(join(ROOT, p), 'utf8');
const dirs = (p) =>
	readdirSync(join(ROOT, p), { withFileTypes: true })
		.filter((e) => e.isDirectory())
		.map((e) => e.name);

/**
 * Screen families that are legitimately not routed, each with its reason.
 *
 * EMPTY, and that is the finding: every candidate exemption checked out as
 * genuinely reachable (desktop `ui`/`ctap`/`executor` are all declared in
 * main.rs; iOS `Gallery` is instantiated at RootView.swift:70). A guard that
 * ships pre-populated with exemptions nobody needs is a guard the next orphan
 * hides behind. An entry here must name the screen and the reason it has no
 * route — never a bare name, and never a whole directory added to make a red
 * check go green.
 */
const EXEMPT = {
	desktop: new Set([]),
	android: new Set([]),
	ios: new Set([])
};

const failures = [];

// --- desktop: a directory under src/ that main.rs never declares ------------
{
	const declared = new Set(
		[...read('app-desktop/vela-wallet/src/main.rs').matchAll(/^mod\s+([a-z_]+);/gm)].map(
			(m) => m[1]
		)
	);
	const present = dirs('app-desktop/vela-wallet/src');
	const orphans = present.filter((d) => !declared.has(d) && !EXEMPT.desktop.has(d));
	if (orphans.length) {
		failures.push(
			`desktop: ${orphans.length} module(s) on disk that src/main.rs never declares, ` +
				`so rustc never compiles them: ${orphans.join(', ')}\n` +
				`         Fix: add \`mod <name>;\` to app-desktop/vela-wallet/src/main.rs ` +
				`and route to it.`
		);
	}
}

// --- Android: a feature package no navigation destination reaches -----------
{
	const nav = read(
		'app-android/vela-wallet/app/src/main/java/app/getvela/wallet/navigation/VelaNavHost.kt'
	);
	const base = 'app-android/vela-wallet/app/src/main/java/app/getvela/wallet/feature';
	const orphans = dirs(base).filter((feature) => {
		if (EXEMPT.android.has(feature)) return false;
		// Reachable if the nav host names the package, or renders any screen from it.
		if (nav.includes(`feature.${feature}.`)) return false;
		const screens = readdirSync(join(ROOT, base, feature))
			.filter((f) => f.endsWith('Screen.kt') || f.endsWith('Sheet.kt'))
			.map((f) => f.replace(/\.kt$/, ''));
		return !screens.some((s) => nav.includes(`${s}(`));
	});
	if (orphans.length) {
		failures.push(
			`android: ${orphans.length} feature package(s) no VelaNavHost destination ` +
				`reaches: ${orphans.join(', ')}\n` +
				`         Fix: add a VelaDestinations route and a composable(...) for each.`
		);
	}
}

// --- iOS: a Features/ folder RootView never instantiates --------------------
{
	const root = read('app-ios/VelaWallet/VelaWallet/App/RootView.swift');
	const base = 'app-ios/VelaWallet/VelaWallet/Features';
	const orphans = dirs(base).filter((feature) => {
		if (EXEMPT.ios.has(feature)) return false;
		const screens = readdirSync(join(ROOT, base, feature))
			.filter((f) => f.endsWith('Screen.swift') || f.endsWith('Sheet.swift'))
			.map((f) => f.replace(/\.swift$/, ''));
		if (!screens.length) return false;
		return !screens.some((s) => root.includes(`${s}(`));
	});
	if (orphans.length) {
		failures.push(
			`ios: ${orphans.length} Features/ folder(s) RootView.swift never ` +
				`instantiates: ${orphans.join(', ')}\n` +
				`     Fix: add a PageOverride.Page case and render the screen from RootView.`
		);
	}
}

if (failures.length) {
	console.error('Unreachable screen families — drawn, compiled or not, and unopenable:\n');
	for (const f of failures) console.error(`  ${f}\n`);
	console.error(
		'Each of these is UI a person cannot get to. If one is intentionally not\n' +
			'routed, add it to EXEMPT in this file WITH the reason — never silently.'
	);
	process.exit(1);
}

console.log('native reachability: every screen family is reachable from its navigation root');
