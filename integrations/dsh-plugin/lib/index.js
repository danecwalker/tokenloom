// @dane/dsh-web-search-tokenloom — a Cordis plugin for DeepSeek Harness.
//
// Registers two providers into the web capability seam (ctx.web):
//
// - search: one `tokenloom search <query> --json --limit N` child per query,
//   mapped onto the seam's source shape from the binary's stable JSON v1
//   output. Engine failures surface as honest provider errors.
// - fetch: one `tokenloom fetch <url> --json` child per retrieval, mapped to
//   the seam's `{ url, statusCode, body: { kind: "text", content }, truncated }`
//   outcome. The body is already-sanitised Markdown (the 7-layer pipeline), so
//   the model-facing tool passes it through verbatim. Non-2xx statuses resolve
//   descriptively; hard failures (SSRF rejection, timeout) throw.
//
// Configuration lives in a `web-search-tokenloom` settings section, editable
// live in Settings → Plugins → "tokenloom" (Schemastery-rendered card), with
// `~/.dsh/profiles/web/cordis.patch.yml` as the patch-layer default and
// `$TOKENLOOM_BIN` as the environment fallback for the binary path.

import { execFile } from "node:child_process";
import z from "@deepseek-ai/schemastery";
import { installSettingsSection, settingsNamespace } from "@deepseek-ai/dsh-settings";
import { launchEnvironmentOf } from "@deepseek-ai/dsh-launch-environment";
import { WebError } from "@deepseek-ai/dsh-web";

/** Stable id both providers register under. */
const TOKENLOOM_PROVIDER_ID = "tokenloom";

/** Default binary name resolved from $PATH. */
const TOKENLOOM_DEFAULT_BIN = "tokenloom";

/** Environment variable naming the binary, read when config omits `bin`. */
const TOKENLOOM_BIN_ENV = "TOKENLOOM_BIN";

/**
 * Settings section schema: mirrored into the Plugins settings card and
 * validated on every patch-layer merge. Numeric fields feed timeouts and
 * limits; `bin` accepts an absolute path or a $PATH name.
 */
const Config = z.object({
	bin: z.string().default(TOKENLOOM_DEFAULT_BIN),
	maxResults: z.number().step(1).min(1).default(10),
	timeoutMs: z.number().step(1).min(1).default(20000),
	fetchTimeoutMs: z.number().step(1).min(1).default(45000)
});

/** Settings namespace carrying this provider's binary and options. */
const TOKENLOOM_SETTINGS_NAMESPACE = settingsNamespace("web-search-tokenloom");

/**
 * Project one resolved settings section into the options the providers serve
 * their next operation with. Environment fallback stays here rather than in
 * the providers: every value they read is already fully defaulted.
 * @param {object} ctx - plugin context supplying the environment plane.
 * @param {object} section - the currently authoritative settings section.
 * @returns options for one search or fetch.
 */
function resolveOptionsFrom(ctx, section) {
	return {
		bin: section.bin ?? launchEnvironmentOf(ctx).get(TOKENLOOM_BIN_ENV)?.value ?? TOKENLOOM_DEFAULT_BIN,
		maxResults: section.maxResults ?? 10,
		timeoutMs: section.timeoutMs ?? 20000,
		fetchTimeoutMs: section.fetchTimeoutMs ?? 45000
	};
}

/**
 * Run one tokenloom invocation and parse its JSON envelope.
 * @param {string} bin - the tokenloom binary to invoke.
 * @param {string[]} args - full argument list for the invocation.
 * @param {number} timeoutMs - hard wall-clock bound for the child process.
 * @param {AbortSignal|undefined} signal - the caller's cancellation signal.
 * @returns {Promise<object>} the parsed JSON envelope.
 */
function runTokenloom(bin, args, timeoutMs, signal) {
	return new Promise((resolve, reject) => {
		const child = execFile(
			bin,
			args,
			{ timeout: timeoutMs, maxBuffer: 16 * 1024 * 1024, windowsHide: true },
			(error, stdout) => {
				if (error !== null) {
					const message = error?.killed
						? `tokenloom timed out after ${timeoutMs}ms`
						: `tokenloom failed: ${error.message}`;
					reject(new WebError(message, "WEB_PROVIDER_ERROR", { cause: error }));
					return;
				}
				try {
					resolve(JSON.parse(stdout));
				} catch (parseError) {
					reject(new WebError(`tokenloom returned unparseable JSON: ${String(parseError)}`, "WEB_PROVIDER_ERROR", { cause: parseError }));
				}
			}
		);
		if (signal !== undefined) {
			if (signal.aborted) child.kill();
			else signal.addEventListener("abort", () => child.kill(), { once: true });
		}
	});
}

/**
 * Map tokenloom's JSON v1 search results to the seam's normalized source
 * shape. Rows without a usable URL are dropped; optional fields stay absent
 * rather than invented, mirroring the seam's contract.
 * @param {object} body - the parsed JSON v1 envelope.
 * @param {number} limit - per-request source cap.
 * @returns {{sources: object[], truncated: boolean}}
 */
function mapSearchResponse(body, limit) {
	const rows = Array.isArray(body?.results) ? body.results : [];
	const sources = [];
	const seen = new Set();
	for (const row of rows) {
		if (sources.length >= limit) break;
		if (typeof row?.url !== "string" || row.url.length === 0) continue;
		if (seen.has(row.url)) continue;
		seen.add(row.url);
		sources.push({
			url: row.url,
			...(typeof row.title === "string" && row.title.length > 0 ? { title: row.title } : {}),
			...(typeof row.snippet === "string" && row.snippet.length > 0 ? { snippet: row.snippet } : {}),
			...(typeof row.published_date === "string" && row.published_date.length > 0 ? { publishedAt: row.published_date } : {})
		});
	}
	return { sources, truncated: body?.total_results > sources.length };
}

/** The tokenloom-backed search provider. */
var TokenloomSearchProvider = class {
	id = TOKENLOOM_PROVIDER_ID;

	/** @param {() => object} resolveOptions - snapshotted once per operation. */
	constructor(resolveOptions) {
		this.resolveOptions = resolveOptions;
	}

	available() {
		return true;
	}

	async search(request, signal) {
		if (signal?.aborted === true) throw new WebError("tokenloom search aborted", "WEB_ABORTED");
		const options = this.resolveOptions();
		const limit = request.maxResults ?? options.maxResults;
		const body = await runTokenloom(options.bin, ["search", request.query, "--json", "--limit", String(limit)], options.timeoutMs, signal);
		const mapped = mapSearchResponse(body, limit);
		if (mapped.sources.length === 0) {
			const failures = Array.isArray(body?.engines_failed) ? body.engines_failed : [];
			const detail = failures.length > 0
				? `; engine failures: ${failures.map((f) => `${f.engine} (${f.error})`).join(", ")}`
				: "";
			throw new WebError(`tokenloom returned no results for this query${detail}`, "WEB_PROVIDER_ERROR");
		}
		return mapped;
	}
};

/**
 * The tokenloom-backed fetch provider: the binary's output is already
 * sanitised Markdown (the 7-layer pipeline), which maps to the seam's
 * `kind: "text"` body — the model-facing tool passes it through verbatim
 * instead of running its own HTML→Markdown conversion. Non-2xx statuses
 * resolve descriptively (a 404 page is a result, not a throw); hard failures
 * (SSRF rejection, timeout) throw.
 */
var TokenloomFetchProvider = class {
	id = TOKENLOOM_PROVIDER_ID;

	constructor(resolveOptions) {
		this.resolveOptions = resolveOptions;
	}

	available() {
		return true;
	}

	async fetch(request, signal) {
		if (signal?.aborted === true) throw new WebError("tokenloom fetch aborted", "WEB_ABORTED");
		const options = this.resolveOptions();
		const body = await runTokenloom(options.bin, ["fetch", request.url, "--json"], options.fetchTimeoutMs, signal);
		return {
			url: typeof body.final_url === "string" && body.final_url.length > 0 ? body.final_url : request.url,
			statusCode: typeof body.status_code === "number" ? body.status_code : 200,
			body: {
				kind: "text",
				content: typeof body.markdown === "string" ? body.markdown : ""
			},
			truncated: body.is_truncated === true
		};
	}
};

/** Cordis plugin name used by loader diagnostics. */
export const name = "web-search-tokenloom";

/** The web seam both providers register into. */
export const inject = ["web"];

/**
 * Register the tokenloom search and fetch providers with `ctx.web`, backed by
 * the live-editable `web-search-tokenloom` settings section. Settings-card
 * edits apply between operations; patch-layer config seeds the section.
 * @param {object} ctx - plugin context.
 * @param {object} config - the patch-layer config for this plugin.
 */
export function apply(ctx, config) {
	let current = () => config;
	installSettingsSection(ctx, TOKENLOOM_SETTINGS_NAMESPACE, Config, config, {
		setSource: (source) => {
			current = source;
		},
		onChange: () => {}
	});
	const resolveOptions = () => resolveOptionsFrom(ctx, current());
	ctx.web.registerSearchProvider(new TokenloomSearchProvider(resolveOptions));
	ctx.web.registerFetchProvider(new TokenloomFetchProvider(resolveOptions));
}

/** Re-exported for tests and diagnostics. */
export { Config, TOKENLOOM_PROVIDER_ID, TOKENLOOM_DEFAULT_BIN, TOKENLOOM_BIN_ENV, TOKENLOOM_SETTINGS_NAMESPACE };
