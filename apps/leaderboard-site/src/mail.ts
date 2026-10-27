// Sending the join code. Production uses Resend's HTTP API with plain fetch;
// tests use a fake; the local server prints the code. Never log codes or emails.

export interface Mailer {
  /** Resolves when the email was accepted; throws when it was not. */
  sendCode(to: string, code: string): Promise<void>;
}

export const DEFAULT_FROM = 'XTrace Leaderboard <noreply@emails.xtrace.ai>';
const RESEND_URL = 'https://api.resend.com/emails';

export function codeEmail(code: string): { subject: string; text: string } {
  return {
    subject: `Your XTrace leaderboard code: ${code}`,
    text: [
      `Your code is ${code}. It expires in 15 minutes.`,
      'Enter it in the XTrace app to join the leaderboard.',
      'If you did not ask to join the XTrace leaderboard, you can ignore this email.',
      '',
    ].join('\n'),
  };
}

export class MailError extends Error {
  override name = 'MailError';
}

/** Used when RESEND_API_KEY is not set: every send fails, so join answers 503. */
export const unavailableMailer: Mailer = {
  async sendCode() {
    throw new MailError('email is not configured');
  },
};

export function resendMailer(options: {
  apiKey: string;
  from?: string | undefined;
  fetch?: typeof fetch;
}): Mailer {
  const send = options.fetch ?? fetch;
  const from = options.from || DEFAULT_FROM;
  return {
    async sendCode(to, code) {
      const { subject, text } = codeEmail(code);
      const response = await send(RESEND_URL, {
        method: 'POST',
        headers: {
          authorization: `Bearer ${options.apiKey}`,
          'content-type': 'application/json',
        },
        body: JSON.stringify({ from, to: [to], subject, text }),
        signal: AbortSignal.timeout(10_000),
      });
      // The status is safe to report; the response body is never read or logged.
      await response.body?.cancel();
      if (!response.ok) throw new MailError(`Resend answered ${response.status}`);
    },
  };
}

/** Local server only: prints the code instead of sending email. */
export function consoleMailer(print: (line: string) => void = console.log): Mailer {
  return {
    async sendCode(to, code) {
      print(`code for ${to}: ${code}`);
    },
  };
}
