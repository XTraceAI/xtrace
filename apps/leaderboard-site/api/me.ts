import { app } from '../src/runtime.js';

export function GET(request: Request): Promise<Response> {
  return app().me.GET(request);
}

export function PATCH(request: Request): Promise<Response> {
  return app().me.PATCH(request);
}

export function DELETE(request: Request): Promise<Response> {
  return app().me.DELETE(request);
}
