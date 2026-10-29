import { app } from '../../src/runtime.js';

export function POST(request: Request): Promise<Response> {
  return app().join.POST(request);
}
