import { app } from '../src/runtime.js';

export function PUT(request: Request): Promise<Response> {
  return app().score.PUT(request);
}
