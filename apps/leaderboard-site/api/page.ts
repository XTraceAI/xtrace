// Served at `/` through the rewrite in vercel.json.
import { app } from '../src/runtime.js';

export function GET(request: Request): Promise<Response> {
  return app().page.GET(request);
}

export function HEAD(request: Request): Promise<Response> {
  return app().page.HEAD(request);
}
