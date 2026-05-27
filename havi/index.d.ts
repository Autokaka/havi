export interface ProxyRule {
  /** Glob pattern matched against full request URL. First-match-wins. */
  pattern: string
  /** Prefix-rewrite target. Replaces the literal prefix of `pattern` (chars before any glob wildcard) with this string. */
  to?: string
  /** Skip remaining proxy rules; let the request hit the network as-is. */
  pass?: boolean
  /** Abort the request. */
  block?: boolean
  /** Synthesize a response with this HTTP status (default 200). */
  status?: number
  /** Inline response body (UTF-8). */
  body?: string
  /** Response headers. Access-Control-Allow-Origin: * is always added. */
  headers?: Record<string, string>
}

export interface RenderOpts {
  source: string
  out?: string
  width?: number
  height?: number
  fps?: number
  duration?: number
  /** Proceed with partial DOM on load timeout instead of erroring. */
  tolerant?: boolean
  /** HTTP proxy rules. First-match-wins. */
  proxy?: ProxyRule[]
}

export interface RenderResult {
  frames: number
  width: number
  height: number
  fps: number
  out: string
  elapsedMs: number
}

export interface ProgressEvent {
  frame: number
  total: number
}

export interface ConsoleEvent {
  level: "info" | "warn" | "error"
  source: string
  message: string
}

export interface Havi {
  render(
    opts: RenderOpts,
    onProgress?: (ev: ProgressEvent) => void,
    onConsole?: (ev: ConsoleEvent) => void,
  ): Promise<RenderResult>
}

export const havi: Havi
