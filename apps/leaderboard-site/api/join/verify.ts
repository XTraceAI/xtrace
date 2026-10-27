import { app } from '../../src/runtime.js';

export function POST(request: Request): Promise<Response> {
  return app().verify.POST(request);
}
