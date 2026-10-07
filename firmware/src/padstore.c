// ===================================================================================
// Keymap storage and host protocol (see padstore.h)
// ===================================================================================
#include "eeprom.h"
#include "padstore.h"

#ifdef SDCC
#define PS_XDATA __xdata
#else
#define PS_XDATA
#endif

static PS_XDATA uint16_t Keymap[LAYER_COUNT][SLOT_COUNT];
PS_XDATA uint8_t PADSTORE_req[16];
PS_XDATA uint8_t PADSTORE_reply[16];
#define req   PADSTORE_req
#define reply PADSTORE_reply
static PS_XDATA uint8_t crcBuf[ACTION_BYTES];

static uint8_t storedCrc(void) {
  uint8_t i;
  for(i = 0; i < ACTION_BYTES; i++) crcBuf[i] = EEPROM_read(ACTION_BASE + i);
  return PAD_crc8(crcBuf, ACTION_BYTES);
}

static uint8_t saveSlot(uint8_t layer, uint8_t slot) {
  uint8_t at = ACTION_BASE + (layer * SLOT_COUNT + slot) * 2;
  uint8_t ok = 1;
  ok &= EEPROM_write(at,     (uint8_t)(Keymap[layer][slot] & 0xFF));
  ok &= EEPROM_write(at + 1, (uint8_t)(Keymap[layer][slot] >> 8));
  return ok;
}

static void loadDefaults(void) {
  uint8_t l, s;
  for(l = 0; l < LAYER_COUNT; l++)
    for(s = 0; s < SLOT_COUNT; s++) Keymap[l][s] = PAD_defaultAction(l, s);
}

static uint8_t saveAll(uint8_t startLayer) {
  uint8_t l, s, ok = 1;
  // Invalidate first, header last: an interrupted save leaves the magic invalid,
  // so the next boot rebuilds from defaults instead of running half a keymap.
  ok &= EEPROM_write(HDR_MAGIC0, 0);
  for(l = 0; l < LAYER_COUNT; l++)
    for(s = 0; s < SLOT_COUNT; s++) ok &= saveSlot(l, s);
  ok &= EEPROM_write(HDR_START_LAYER, startLayer);
  ok &= EEPROM_write(HDR_CRC,         storedCrc());
  ok &= EEPROM_write(HDR_COUNT,       SLOT_COUNT);
  ok &= EEPROM_write(HDR_VERSION,     CFG_VERSION);
  ok &= EEPROM_write(HDR_MAGIC1,      CFG_MAGIC1);
  ok &= EEPROM_write(HDR_MAGIC0,      CFG_MAGIC0);
  return ok;
}

static uint8_t loadStored(void) {
  uint8_t l, s, at;
  uint16_t v;
  if(EEPROM_read(HDR_MAGIC0)  != CFG_MAGIC0)  return 0;
  if(EEPROM_read(HDR_MAGIC1)  != CFG_MAGIC1)  return 0;
  if(EEPROM_read(HDR_VERSION) != CFG_VERSION) return 0;
  if(EEPROM_read(HDR_COUNT)   != SLOT_COUNT)  return 0;
  if(EEPROM_read(HDR_START_LAYER) >= LAYER_COUNT) return 0;
  if(EEPROM_read(HDR_CRC)     != storedCrc()) return 0;
  for(l = 0; l < LAYER_COUNT; l++) {
    for(s = 0; s < SLOT_COUNT; s++) {
      at = ACTION_BASE + (l * SLOT_COUNT + s) * 2;
      v = EEPROM_read(at) | ((uint16_t)EEPROM_read(at + 1) << 8);
      // The CRC proves the bytes are intact, not that they mean anything.
      if(!PAD_validPacked(v)) return 0;
      Keymap[l][s] = v;
    }
  }
  return 1;
}

uint8_t PADSTORE_init(void) {
  if(loadStored()) return EEPROM_read(HDR_START_LAYER);
  loadDefaults();
  saveAll(0);
  return 0;
}

uint16_t PAD_keymap(uint8_t layer, uint8_t slot) {
  if(layer >= LAYER_COUNT || slot >= SLOT_COUNT) return 0;
  return Keymap[layer][slot];
}

uint8_t PADSTORE_handle(void) {
  uint8_t i, idx, layer, type, mod, boot = 0;
  uint16_t code, packed;

  for(i = 0; i < 16; i++) reply[i] = 0;
  reply[0] = CFG_REPORT_ID;
  reply[1] = req[1];                         // echo the command
  idx = req[2];
  layer = req[7];

  switch(req[1]) {
    case CMD_GET_INFO:
      reply[2]  = CFG_MAGIC0;
      reply[3]  = CFG_MAGIC1;
      reply[4]  = CFG_VERSION;
      reply[5]  = SLOT_COUNT;
      reply[6]  = EEPROM_SIZE;
      reply[7]  = ST_OK;
      reply[8]  = FW_VERSION_MAJOR;
      reply[9]  = FW_VERSION_MINOR;
      reply[10] = FW_VERSION_PATCH;
      reply[11] = LAYER_COUNT;
      reply[12] = PAD_activeLayer(&Pad.layers);
      reply[13] = PAD_rawActive(&Pad.raw);
      reply[14] = EEPROM_read(HDR_START_LAYER);
      break;

    case CMD_GET_ACTION:
      reply[2] = idx;
      reply[8] = layer;
      if(idx >= SLOT_COUNT || layer >= LAYER_COUNT) { reply[7] = ST_BAD_INDEX; break; }
      PAD_decode(Keymap[layer][idx], &type, &mod, &code);
      reply[3] = type;
      reply[4] = mod;
      reply[5] = (uint8_t)(code & 0xFF);
      reply[6] = (uint8_t)(code >> 8);
      reply[7] = ST_OK;
      break;

    case CMD_SET_ACTION:
      reply[2] = idx;
      reply[8] = layer;
      if(idx >= SLOT_COUNT || layer >= LAYER_COUNT) { reply[7] = ST_BAD_INDEX; break; }
      type = req[3];
      mod  = req[4];
      code = req[5] | ((uint16_t)req[6] << 8);
      if(!PAD_encode(type, mod, code, &packed)) { reply[7] = ST_BAD_ACTION; break; }
      Keymap[layer][idx] = packed;
      reply[7] = (saveSlot(layer, idx) && EEPROM_write(HDR_CRC, storedCrc()))
                 ? ST_OK : ST_WRITE_FAIL;
      break;

    case CMD_RESET:
      loadDefaults();                           // held keys still release what they pressed
      PAD_setLayer(0);
      reply[7] = saveAll(0) ? ST_OK : ST_WRITE_FAIL;
      break;

    case CMD_DUMP:
      reply[2] = idx;
      if(idx >= EEPROM_SIZE) { reply[15] = ST_BAD_INDEX; break; }
      for(i = 0; i < 12 && idx + i < EEPROM_SIZE; i++) reply[3 + i] = EEPROM_read(idx + i);
      reply[15] = ST_OK;                        // status after the 12 data bytes
      break;

    case CMD_CORRUPT:
      reply[7] = EEPROM_write(HDR_CRC, EEPROM_read(HDR_CRC) ^ 0xFF)
                 ? ST_OK : ST_WRITE_FAIL;
      break;

    case CMD_RAW_MODE:
      reply[7] = PAD_setRaw(req[2] | ((uint16_t)req[3] << 8)) ? ST_OK : ST_BAD_ARG;
      reply[8] = PAD_rawActive(&Pad.raw);
      break;

    case CMD_SET_LAYER:
      reply[8] = idx;
      if(idx >= LAYER_COUNT || req[3] > 1) { reply[7] = ST_BAD_ARG; break; }
      PAD_setLayer(idx);
      reply[7] = (!req[3] || EEPROM_write(HDR_START_LAYER, idx)) ? ST_OK : ST_WRITE_FAIL;
      break;

    case CMD_GET_STATS: {
      // Page 0: illegal transitions per knob, overruns, queue drops, max queue.
      // Page 1: detents decoded per knob, cw and ccw. Little-endian u16 each,
      // in bytes 3..6 and 8..15 (byte 7 is the status, as everywhere).
      static PS_XDATA padstats_t st;
      static PS_XDATA uint16_t v[6];
      reply[2] = idx;
      if(idx > 1 || req[3] > 1) { reply[7] = ST_BAD_ARG; break; }
      PAD_getStats(&st, req[3]);
      if(idx == 0) {
        v[0] = st.illegal[0]; v[1] = st.illegal[1]; v[2] = st.illegal[2];
        v[3] = st.overruns;   v[4] = st.queueDrops; v[5] = st.maxQueue;
      } else {
        v[0] = st.cw[0]; v[1] = st.ccw[0]; v[2] = st.cw[1];
        v[3] = st.ccw[1]; v[4] = st.cw[2]; v[5] = st.ccw[2];
      }
      for(i = 0; i < 12; i++) {                 // data bytes 3..6, then 8..15
        reply[i < 4 ? 3 + i : 4 + i] = (uint8_t)((i & 1) ? (v[i >> 1] >> 8) : (v[i >> 1] & 0xFF));
      }
      reply[7] = ST_OK;
      break;
    }

    case CMD_BOOTLOADER:
      if(req[2] != 'B' || req[3] != 'L') { reply[7] = ST_BAD_ARG; break; }
      reply[7] = ST_OK;
      boot = 1;                                 // the caller answers, then jumps
      break;

    default:
      reply[7] = ST_UNKNOWN;
      break;
  }

  return boot;
}
