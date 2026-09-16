import { useCallback, useEffect, useRef, useState } from "react";
import type {
  DesignAdvice,
  DraftUpdate,
  DraftView,
  ThemeControls,
  StyleSelection,
} from "./types";
import profiles from "../shared/offline-styles.json";
import legacyProfiles from "../shared/offline-styles-v1.json";
import { errorText, type StudioApi } from "./studio-api";

type Values = {
  name: string;
  controls: ThemeControls;
  design: DesignAdvice | null;
  style: StyleSelection | null;
};
type Session = {
  view: DraftView;
  values: Values;
  version: number;
  committed: number;
  running: boolean;
  resetVersion: number;
  error: string | null;
};
const valuesOf = (view: DraftView): Values => ({
  name: view.document.name,
  controls: view.document.controls,
  design: view.document.design ?? null,
  style: view.document.style ?? null,
});
const validValues = (values: Values) =>
  values.name.trim().length > 0 &&
  values.name.trim().length <= 48 &&
  (!values.controls.accent || /^#[\da-fA-F]{6}$/.test(values.controls.accent));

export function useThemeEditor(api: StudioApi) {
  const current = useRef<Session | null>(null);
  const [wake, setWake] = useState(0);
  const [state, setState] = useState<{
    draft: DraftView | null;
    fields: DraftUpdate | null;
    design: DesignAdvice | null;
    pending: boolean;
    error: string | null;
  }>({ draft: null, fields: null, design: null, pending: false, error: null });
  const publish = useCallback((session: Session) => {
    if (current.current !== session) return;
    setState({
      draft: session.view,
      fields: {
        name: session.values.name,
        controls: session.values.controls,
        sequence: session.view.document.editSequence,
        style: session.values.style,
      },
      design: session.values.design,
      pending: session.running || session.version !== session.committed,
      error: session.error,
    });
  }, []);
  const adopt = useCallback(
    (view: DraftView) => {
      const session: Session = {
        view,
        values: valuesOf(view),
        version: 0,
        committed: 0,
        running: false,
        resetVersion: 0,
        error: null,
      };
      current.current = session;
      publish(session);
      setWake((value) => value + 1);
    },
    [publish],
  );
  const change = useCallback(
    (patch: Partial<Values>, reset = false) => {
      const session = current.current;
      if (!session) return;
      session.values = { ...session.values, ...patch };
      session.version += 1;
      if (reset) session.resetVersion = session.version;
      session.error = null;
      publish(session);
      setWake((value) => value + 1);
    },
    [publish],
  );
  const chooseStyle = useCallback(
    (style: StyleSelection) => {
      const profile = [...profiles, ...legacyProfiles].find(
        (p) => p.id === style.id && p.version === style.version,
      );
      if (!profile) return;
      change({ style, controls: { ...profile.controls } }, true);
    },
    [change],
  );
  const changeDesign = useCallback(
    (design: DesignAdvice) => change({ design }),
    [change],
  );
  const retry = useCallback(() => {
    const session = current.current;
    if (!session || session.running) return;
    session.error = null;
    publish(session);
    setWake((value) => value + 1);
  }, [publish]);

  const flush = useCallback(
    async (session: Session) => {
      if (
        session.running ||
        session.error ||
        current.current !== session ||
        session.version === session.committed ||
        !validValues(session.values)
      )
        return;
      session.running = true;
      const target = session.values;
      const version = session.version;
      const id = session.view.document.draftId;
      publish(session);
      try {
        // Exactly one sequence and one persistence operation for the complete edit.
        const view = await api.updateDraft(id, {
          name: target.name,
          controls: target.controls,
          design: target.design,
          style: target.style,
          resetStyle: session.resetVersion > session.committed,
          sequence: session.view.document.editSequence + 1,
        });
        if (current.current !== session) return;
        // Rebase untouched regions onto a completed style reset without losing edits
        // made while the compiler was running.
        if (session.version !== version && view.document.design) {
          const rebased = structuredClone(view.document.design);
          const before = target.design,
            latest = session.values.design;
          if (before && latest) {
            for (const key of ["backgroundX", "backgroundY", "veil"] as const)
              if (latest[key] !== before[key]) rebased[key] = latest[key];
            for (const region of Object.keys(latest.regions) as Array<
              keyof DesignAdvice["regions"]
            >) {
              const patch = Object.fromEntries(
                Object.entries(latest.regions[region]).filter(
                  ([key, value]) =>
                    value !==
                    before.regions[region][
                      key as keyof (typeof before.regions)[typeof region]
                    ],
                ),
              );
              rebased.regions[region] = {
                ...rebased.regions[region],
                ...patch,
              };
            }
          }
          session.values = { ...session.values, design: rebased };
        }
        session.view = view;
        session.committed = version;
        if (session.version === version)
          session.values = valuesOf(session.view);
      } catch (error) {
        if (current.current === session) session.error = errorText(error);
      } finally {
        session.running = false;
        if (current.current === session) {
          // A later edit is a new attempt, but a failed latest edit never loops.
          if (session.version !== version) {
            session.error = null;
            setWake((value) => value + 1);
          }
          publish(session);
        }
      }
    },
    [api, publish],
  );
  useEffect(() => {
    const session = current.current;
    if (!session) return;
    const timer = window.setTimeout(() => void flush(session), 100);
    return () => window.clearTimeout(timer);
  }, [wake, flush]);
  useEffect(
    () => () => {
      current.current = null;
    },
    [],
  );

  return {
    ...state,
    adopt,
    change,
    changeDesign,
    chooseStyle,
    retry,
    valid:
      !!state.fields &&
      validValues({
        ...state.fields,
        design: state.design,
        style: state.fields.style ?? null,
      }),
  };
}
