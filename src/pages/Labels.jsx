import React, { useEffect, useState } from 'react';
import QRCode from 'qrcode';
import { Printer } from 'lucide-react';
import { useAuth } from '../auth.jsx';
import { ErrorBanner, Modal } from '../components.jsx';

// What a label's QR code holds: a link that opens the asset in DeviceTrustline. Scanners in the app
// also accept the bare asset tag, so labels keep working if the domain changes.
export const labelUrl = (assetTag) => `${window.location.origin}/?asset=${encodeURIComponent(assetTag)}`;

export function LabelSheet({ assets, onClose }) {
  const { user } = useAuth();
  const [codes, setCodes] = useState(null);
  const [error, setError] = useState(null);

  useEffect(() => {
    Promise.all(assets.map((a) => QRCode.toDataURL(labelUrl(a.assetTag), { margin: 0, width: 240, errorCorrectionLevel: 'M' })))
      .then(setCodes)
      .catch(setError);
  }, [assets]);

  return (
    <Modal title={`Print ${assets.length} label${assets.length === 1 ? '' : 's'}`}
      subtitle="Each label has a QR code that opens the asset when scanned. Use plain or adhesive paper." onClose={onClose} wide>
      <ErrorBanner error={error} />
      <div className="modal-actions top">
        <button className="primary" disabled={!codes} onClick={() => window.print()}><Printer size={16} /> Print</button>
      </div>
      <div className="label-sheet">
        {assets.map((a, i) => (
          <div className="label" key={a.id}>
            {codes ? <img src={codes[i]} alt={`QR code for ${a.assetTag}`} /> : <div className="qr-placeholder" />}
            <div>
              <strong>{a.assetTag}</strong>
              <span>{a.name}</span>
              <small>{user.organization.name}</small>
            </div>
          </div>
        ))}
      </div>
    </Modal>
  );
}
