import { useEffect, useId, useRef, useState } from 'react';
import { useData } from '../../data/DataProvider';
import type { GradingLinkFound } from '../../data/generated/GradingLinkFound';
import type { GradingLinkSkill } from '../../data/generated/GradingLinkSkill';
import { Button } from '../../kit/Button';
import {
  KEEP_FAILED,
  LINK_HELP,
  LINK_KEEP_NOTE,
  LOOKUP_FAILED,
  linkKeepStateText,
  linkKeepText,
  linkLookupText,
  linkRubricText,
  linkSizeText,
  linkSourceText,
  linkWhereText,
  useRefreshInstalled,
} from './scores';

/**
 * "Add from a link", at the top of the Add a grading skill dialog: a field for a
 * public GitHub link and Look up. The native side reads the link, downloads
 * that commit and answers the skills it holds, each with whether it can be
 * kept (`GradingLinkLookup`); this shows the preview, lets the user choose
 * among several (none chosen at first), and sends Keep with the look-up and
 * the chosen paths it named. Closing the dialog cancels a look-up going and
 * drops the preview.
 */
export function AddFromLink() {
  const { source } = useData();
  const grading = source.grading;
  const refresh = useRefreshInstalled();
  const heading = useId();
  const help = useId();
  const [text, setText] = useState('');
  const [looking, setLooking] = useState(false);
  const [preview, setPreview] = useState<GradingLinkFound | null>(null);
  const [chosen, setChosen] = useState<string[]>([]);
  const [keeping, setKeeping] = useState(false);
  const [message, setMessage] = useState<{ text: string; tone: 'alert' | 'status' } | null>(null);
  const [focusMessage, setFocusMessage] = useState(false);
  const status = useRef<HTMLParagraphElement>(null);
  // Only the latest Look up's answer is shown.
  const asked = useRef(0);
  // What closing the dialog must end: a look-up going, and the look-up the
  // native side still holds (its preview and downloaded files) until a keep
  // ends it, whatever this view shows.
  const going = useRef(false);
  const held = useRef<string | null>(null);
  const alive = useRef(true);
  useEffect(() => {
    going.current = looking;
  });
  useEffect(() => {
    alive.current = true;
    return () => {
      alive.current = false;
      if (!grading) return;
      if (going.current) void grading.linkCancel().catch(() => false);
      if (held.current) void grading.linkDiscard(held.current).catch(() => false);
      held.current = null;
    };
  }, [grading]);
  useEffect(() => {
    if (!focusMessage) return;
    setFocusMessage(false);
    status.current?.focus();
  }, [focusMessage]);
  const link = text.trim();
  const lookUp = async () => {
    if (!grading || link === '' || looking) return;
    const ask = ++asked.current;
    // A new look-up replaces the one held natively.
    held.current = null;
    setLooking(true);
    setPreview(null);
    setChosen([]);
    setMessage(null);
    let shown: { text: string; tone: 'alert' | 'status' } | null;
    let found: GradingLinkFound | null = null;
    try {
      const answer = await grading.linkLookup(link);
      if (answer.state === 'found') {
        // Answered after the dialog closed, or after a newer Look up: drop it.
        if (!alive.current || ask !== asked.current) {
          void grading.linkDiscard(answer.lookup_id).catch(() => false);
          return;
        }
        held.current = answer.lookup_id;
        found = answer;
        shown = null;
      } else {
        shown = {
          text: linkLookupText(answer),
          tone: answer.state === 'cancelled' ? 'status' : 'alert',
        };
      }
    } catch {
      shown = { text: LOOKUP_FAILED, tone: 'alert' };
    }
    if (ask !== asked.current) return;
    setLooking(false);
    setPreview(found);
    setMessage(shown);
  };
  const cancel = () => {
    if (!grading || !looking) return;
    void grading.linkCancel().catch(() => false);
  };
  const keep = async (paths: string[]) => {
    if (!grading || !preview || keeping) return;
    setKeeping(true);
    setMessage(null);
    let shown: string;
    let next: GradingLinkFound | null = preview;
    try {
      const answer = await grading.linkKeep(preview.lookup_id, paths);
      shown = linkKeepText(answer);
      if (answer.state === 'changed') {
        // The preview as it is now; a choice that can't be kept is dropped.
        next = answer;
        setChosen((current) =>
          current.filter((path) =>
            answer.skills.some((skill) => skill.path === path && skill.keep.state === 'keepable'),
          ),
        );
      } else if (
        answer.state === 'done' ||
        answer.state === 'expired' ||
        answer.state === 'not_configured'
      ) {
        // The native side holds it no more.
        if (held.current === preview.lookup_id) held.current = null;
        next = null;
      }
      if (answer.state === 'done') refresh();
    } catch {
      shown = KEEP_FAILED;
    }
    setKeeping(false);
    setPreview(next);
    if (next === null) {
      setChosen([]);
      setText('');
    }
    setMessage({ text: shown, tone: 'status' });
    // The Keep button is gone when the preview is: say what happened there.
    if (next === null) setFocusMessage(true);
  };
  return (
    <section className="xt-score-link" aria-labelledby={heading}>
      <h3 id={heading}>Add from a link</h3>
      <form
        className="xt-score-link-form"
        onSubmit={(event) => {
          event.preventDefault();
          void lookUp();
        }}
      >
        <input
          type="text"
          inputMode="url"
          autoComplete="off"
          spellCheck={false}
          aria-label="GitHub link"
          aria-describedby={help}
          placeholder="https://github.com/owner/repository"
          value={text}
          onChange={(event) => setText(event.target.value)}
        />
        <Button
          type="submit"
          variant="outline"
          height={28}
          aria-disabled={link === '' || looking || undefined}
        >
          {looking ? 'Looking up…' : 'Look up'}
        </Button>
        {looking && (
          <Button variant="ghost" height={28} onClick={cancel}>
            Cancel
          </Button>
        )}
      </form>
      <p id={help} className="xt-score-link-help">
        {LINK_HELP}
      </p>
      {/* Focusable by script only: focus lands here when Keep is gone. */}
      <p ref={status} className="xt-scores-message" role="status" tabIndex={-1}>
        {looking ? `Looking up ${link}…` : message?.tone === 'status' ? message.text : null}
      </p>
      <p className="xt-scores-message" role="alert">
        {!looking && message?.tone === 'alert' ? message.text : null}
      </p>
      {preview && (
        <Preview
          preview={preview}
          chosen={chosen}
          keeping={keeping}
          onChoose={(path, on) =>
            setChosen((current) =>
              on ? [...current, path] : current.filter((other) => other !== path),
            )
          }
          onKeep={(paths) => void keep(paths)}
        />
      )}
    </section>
  );
}

function Preview({
  preview,
  chosen,
  keeping,
  onChoose,
  onKeep,
}: {
  preview: GradingLinkFound;
  chosen: string[];
  keeping: boolean;
  onChoose: (path: string, on: boolean) => void;
  onKeep: (paths: string[]) => void;
}) {
  const several = preview.skills.length > 1;
  const only = preview.skills[0];
  // In the preview's order, whatever order they were ticked in.
  const paths = preview.skills
    .filter((skill) => chosen.includes(skill.path))
    .map((skill) => skill.path);
  return (
    <div className="xt-score-link-preview" data-several={several || undefined}>
      <p className="xt-score-link-source">
        <strong>{linkSourceText(preview.source)}</strong>
        <span title={preview.source.commit}>{linkWhereText(preview.source)}</span>
      </p>
      <div
        className="xt-score-link-list"
        role="region"
        aria-label="Skills at this link"
        tabIndex={0}
      >
        <ul role="list" aria-label="Skills at this link">
          {preview.skills.map((skill) => (
            <LinkRow
              key={skill.path}
              skill={skill}
              choosable={several}
              chosen={chosen.includes(skill.path)}
              keeping={keeping}
              onChoose={(on) => onChoose(skill.path, on)}
            />
          ))}
        </ul>
      </div>
      {preview.incomplete && (
        <p className="xt-scores-note">
          This repository has more skills than are listed; link a folder inside it to see others.
        </p>
      )}
      <div className="xt-score-link-keep">
        <p className="xt-scores-note">{LINK_KEEP_NOTE}</p>
        {several
          ? preview.keepable > 0 && (
              <Button
                variant="primary"
                height={28}
                aria-disabled={paths.length === 0 || keeping || undefined}
                onClick={() => paths.length > 0 && !keeping && onKeep(paths)}
              >
                {keeping
                  ? 'Keeping…'
                  : paths.length === 0
                    ? 'Keep chosen skills'
                    : paths.length === 1
                      ? 'Keep 1 skill'
                      : `Keep ${paths.length} skills`}
              </Button>
            )
          : only?.keep.state === 'keepable' && (
              <Button
                variant="primary"
                height={28}
                aria-label={`Keep ${only.name}`}
                aria-disabled={keeping || undefined}
                onClick={() => !keeping && onKeep([only.path])}
              >
                {keeping ? 'Keeping…' : 'Keep'}
              </Button>
            )}
      </div>
    </div>
  );
}

function LinkRow({
  skill,
  choosable,
  chosen,
  keeping,
  onChoose,
}: {
  skill: GradingLinkSkill;
  /** Several skills: each keepable one has a checkbox. */
  choosable: boolean;
  chosen: boolean;
  keeping: boolean;
  onChoose: (on: boolean) => void;
}) {
  const blocked = linkKeepStateText(skill.keep);
  const box = choosable && skill.keep.state === 'keepable';
  const name = (
    <span className="xt-score-add-name" title={skill.path || undefined}>
      {skill.name}
    </span>
  );
  return (
    <li className="xt-score-link-row" data-path={skill.path} data-keep={skill.keep.state}>
      {box ? (
        <label className="xt-score-link-choice">
          <input
            type="checkbox"
            // WebKit skips a checkbox on Tab without an explicit tab stop.
            tabIndex={0}
            checked={chosen}
            disabled={keeping}
            onChange={(event) => onChoose(event.target.checked)}
          />
          {name}
        </label>
      ) : (
        name
      )}
      <span className="xt-score-add-source">{linkSizeText(skill.files, skill.bytes)}</span>
      <span className="xt-score-add-description" title={skill.description || undefined}>
        {skill.description || 'No description'}
      </span>
      <span
        className="xt-score-add-reason"
        data-tone={skill.rubric.state === 'problem' ? undefined : 'muted'}
      >
        {linkRubricText(skill.rubric)}
      </span>
      {skill.folder_name !== null && skill.keep.state === 'keepable' && (
        <span className="xt-score-add-reason" data-tone="muted">
          {`Folder name: ${skill.folder_name}`}
        </span>
      )}
      {blocked && <span className="xt-score-add-reason">{blocked}</span>}
    </li>
  );
}
