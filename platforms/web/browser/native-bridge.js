// SPDX-License-Identifier: GPL-2.0-or-later
// Both Ruby bridge encodings must apply the same browser-side effects.
export function createNativeDispatcher(engine, onTextInputMode) {
  const decoder = new TextDecoder();
  return (request) => {
    const response = engine.web_native_call(request);
    // Avoid decoding ordinary rendering commands on the hot bridge path.
    if (request[0] === 105 && request[6] === 116) {
      const [operation, enabled] = decoder.decode(request.subarray(0, 48)).split(/[\t\n]/);
      if (operation === 'input_text_mode_set') onTextInputMode(Number(enabled) !== 0);
    }
    return response;
  };
}
