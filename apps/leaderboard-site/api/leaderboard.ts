import { app } from '../src/runtime.js';

export function GET(request: Request): Promise<Response> {
  return app().leaderboard.GET(request);
}
