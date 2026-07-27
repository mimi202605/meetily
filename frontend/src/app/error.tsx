'use client';

import { useEffect } from 'react';

export default function Error({
  error,
  reset,
}: {
  error: Error & { digest?: string };
  reset: () => void;
}) {
  useEffect(() => {
    console.error('[PageError]', error);
  }, [error]);

  return (
    <div style={{
      display: 'flex',
      flexDirection: 'column',
      alignItems: 'center',
      justifyContent: 'center',
      minHeight: '100vh',
      padding: '40px',
      fontFamily: 'system-ui, sans-serif',
      background: '#f9fafb',
    }}>
      <div style={{ maxWidth: '600px', width: '100%' }}>
        <h2 style={{ color: '#dc2626', marginBottom: '16px' }}>
          页面内容加载失败
        </h2>
        <p style={{ color: '#4b5563', marginBottom: '24px' }}>
          右侧面板渲染时遇到错误。请尝试重新加载。
        </p>
        <pre style={{
          background: '#f3f4f6',
          padding: '16px',
          borderRadius: '8px',
          fontSize: '13px',
          overflow: 'auto',
          color: '#374151',
          marginBottom: '24px',
          maxHeight: '300px',
          whiteSpace: 'pre-wrap',
          wordBreak: 'break-all',
        }}>
          {error?.message || 'Unknown error'}
          {error?.digest ? '\nDigest: ' + error.digest : ''}
        </pre>
        <button
          onClick={() => reset()}
          style={{
            padding: '10px 24px',
            background: '#111827',
            color: 'white',
            border: 'none',
            borderRadius: '6px',
            cursor: 'pointer',
            fontSize: '14px',
            boxShadow: '0 2px 4px rgba(0,0,0,0.1)',
          }}
        >
          重新加载
        </button>
      </div>
    </div>
  );
}
