import React, { useEffect, useRef, useState } from 'react';
import { Camera, CameraOff } from 'lucide-react';

// Camera-based QR/barcode reader. The decoder is only downloaded when the camera is switched on.
export function CameraScanner({ onCode }) {
  const [on, setOn] = useState(false);
  const [error, setError] = useState(null);
  const videoRef = useRef(null);
  const onCodeRef = useRef(onCode);
  onCodeRef.current = onCode;

  useEffect(() => {
    if (!on) return undefined;
    let controls;
    let stopped = false;
    (async () => {
      try {
        const { BrowserMultiFormatReader } = await import('@zxing/browser');
        const reader = new BrowserMultiFormatReader();
        controls = await reader.decodeFromConstraints(
          { video: { facingMode: 'environment' } },
          videoRef.current,
          (result) => { if (result) onCodeRef.current(result.getText()); },
        );
        if (stopped) controls.stop();
      } catch (err) {
        setError(err?.name === 'NotAllowedError'
          ? 'Camera permission was denied. Allow camera access for this site, or use a handheld scanner.'
          : 'No camera is available on this device. Use a handheld scanner or type the code.');
        setOn(false);
      }
    })();
    return () => { stopped = true; controls?.stop(); };
  }, [on]);

  return (
    <>
      <div className="camera-row">
        <button className="outline" type="button" onClick={() => { setError(null); setOn(!on); }}>
          {on ? <><CameraOff size={16} /> Stop camera</> : <><Camera size={16} /> Use camera</>}
        </button>
        {error && <span className="camera-error">{error}</span>}
      </div>
      {on && <div className="video-wrap"><video ref={videoRef} muted playsInline /><div className="reticle" /></div>}
    </>
  );
}

// Drops repeat reads of the same code within a short window (cameras report a code many times a second).
export function useDedupe(windowMs = 2500) {
  const last = useRef({ code: '', at: 0 });
  return (code) => {
    const now = Date.now();
    if (code === last.current.code && now - last.current.at < windowMs) return false;
    last.current = { code, at: now };
    return true;
  };
}
