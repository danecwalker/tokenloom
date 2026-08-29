// @dane/dsh-web-search-tokenloom — a Cordis plugin for DeepSeek Harness.
//
// Registers a "tokenloom" search provider into the web capability seam
// (ctx.web). One child process per search: `tokenloom search <query> --json
// --limit N` — the binary's stable JSON v1 output is mapped onto the seam's
// source shape (url / title / snippet / publishedAt). Engine failures are
// surfaced as provider errors with the binary's own honest diagnostics
// attached.
//
// Installation & wiring: see the repository README ("DeepSeek Harness"
// section) — the provider is selected via the `web` row's searchProvider
// override in ~/.dsh/profiles/web/cordis.patch.yml.

import { execFile } from "node:child_process";
import { WebError } from "@deepseek-ai/dsh-web";

/** Stable id this provider registers under (the `web` seam's `searchProvider`). */
const TOKENLOOM_PROVIDER_ID = "tokenloom";

/** Default binary name resolved from $PATH. */
const TOKENLOOM_DEFAULT_BIN = "tokenloom";

/** Environment variable naming the binary, read when config omits `bin`. */
const TOKENLOOM_BIN_ENV = "TOKENLOOM_BIN";

/**
 * Run one `tokenloom search` and parse its JSON v1 envelope.
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
					const message = typeof error?.killSignal === "string" || error?.killed
						? `tokenloom search timed out after ${timeoutMs}ms`
						: `tokenloom search failed: ${error.message}`;
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
 * Map tokenloom's JSON v1 results to the seam's normalized source shape.
 * Rows without a usable URL are dropped; optional fields stay absent rather
 * than invented, mirroring the seam's contract.
 * @param {object} body - the parsed JSON v1 envelope.
 * @param {number} limit - per-request source cap.
 * @returns {{sources: object[], truncated: boolean}}
 */
function mapResponse(body, limit) {
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
		const args = ["search", request.query, "--json", "--limit", String(limit)];
		const body = await runTokenloom(options.bin, args, options.timeoutMs, signal);
		const mapped = mapResponse(body, limit);
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
 * The tokenloom-backed fetch provider: one `tokenloom fetch <url> --json`
 * child per retrieval. The binary returns already-sanitised Markdown (the
 * 7-layer pipeline), which maps to the seam's `kind: "text"` body — the tool
 * passes it through verbatim instead of running its own HTML→Markdown
 * conversion. Non-2xx statuses resolve descriptively (a 404 page is a
 * result, not a throw); hard failures (SSRF rejection, timeout) throw.
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
		const args = ["fetch", request.url, "--json"];
		const body = await runTokenloom(options.bin, args, options.fetchTimeoutMs, signal);
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

/** The web seam this provider registers into. */
export const inject = ["web"];

/**
 * Register the tokenloom search and fetch providers with `ctx.web`.
 * @param {object} ctx - plugin context.
 * @param {{bin?: string, maxResults?: number, timeoutMs?: number}} config - the
 * patch-layer config for this plugin (static defaults; $TOKENLOOM_BIN is the
 * environment fallback for the binary path).
 */
export function apply(ctx, config) {
	const configured = config ?? {};
	const resolveOptions = () => ({
		bin: configured.bin ?? process.env[TOKENLOOM_BIN_ENV] ?? TOKENLOOM_DEFAULT_BIN,
		maxResults: configured.maxResults ?? 10,
		timeoutMs: configured.timeoutMs ?? 20000,
		fetchTimeoutMs: configured.fetchTimeoutMs ?? 45000
	});
	ctx.web.registerSearchProvider(new TokenloomSearchProvider(resolveOptions));
	ctx.web.registerFetchProvider(new TokenloomFetchProvider(resolveOptions));
}
