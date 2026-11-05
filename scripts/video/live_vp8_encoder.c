/* Bounded synthetic fixture source, not a media authorization boundary.
 * stdin: original frame index u32le, flags u32le (bit 0 requests a keyframe).
 * stdout: "V8F1", index u32le, length u32le, key u32le, encoded bytes.
 * The owning process must bound pipe IO, terminate and join this child.
 */
#ifdef __APPLE__
#define _DARWIN_C_SOURCE
#else
#define _POSIX_C_SOURCE 200809L
#endif
#include <errno.h>
#include <fcntl.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>
#include <unistd.h>
#include <vpx/vp8cx.h>
#include <vpx/vpx_encoder.h>
#include <vpx/vp8dx.h>
#include <vpx/vpx_decoder.h>

enum { WIDTH = 640, HEIGHT = 360, FPS = 30, MAX_FRAMES = 62 * FPS,
       RAW_BYTES = WIDTH * HEIGHT * 3 / 2, MAX_ENCODED = 256 * 1024 };

static uint32_t load32(const unsigned char *p) {
  return (uint32_t)p[0] | (uint32_t)p[1] << 8 | (uint32_t)p[2] << 16 |
         (uint32_t)p[3] << 24;
}
static void save32(unsigned char *p, uint32_t v) {
  for (unsigned n = 0; n < 4; ++n) p[n] = (unsigned char)(v >> (8 * n));
}
static int same_file(const struct stat *a, const struct stat *b) {
#ifdef __APPLE__
  return a->st_dev == b->st_dev && a->st_ino == b->st_ino &&
         a->st_size == b->st_size && a->st_mode == b->st_mode &&
         a->st_mtimespec.tv_sec == b->st_mtimespec.tv_sec &&
         a->st_mtimespec.tv_nsec == b->st_mtimespec.tv_nsec &&
         a->st_ctimespec.tv_sec == b->st_ctimespec.tv_sec &&
         a->st_ctimespec.tv_nsec == b->st_ctimespec.tv_nsec;
#else
  return a->st_dev == b->st_dev && a->st_ino == b->st_ino &&
         a->st_size == b->st_size && a->st_mode == b->st_mode &&
         a->st_mtim.tv_sec == b->st_mtim.tv_sec &&
         a->st_mtim.tv_nsec == b->st_mtim.tv_nsec &&
         a->st_ctim.tv_sec == b->st_ctim.tv_sec &&
         a->st_ctim.tv_nsec == b->st_ctim.tv_nsec;
#endif
}
static int read_frame(int fd, uint32_t index, unsigned char *raw) {
  size_t done = 0;
  while (done < RAW_BYTES) {
    ssize_t n = pread(fd, raw + done, RAW_BYTES - done,
                      (off_t)index * RAW_BYTES + (off_t)done);
    if (n < 0 && errno == EINTR) continue;
    if (n <= 0) return 0;
    done += (size_t)n;
  }
  return 1;
}
/* One actual receiver decoder. No PLC, frame fabrication, clock rewrite, or
 * per-frame decoder restart. Complete bounded RTP assembly belongs to caller.
 * stdin: "D8F1", request ordinal, encoded length, actual keyframe bit, bytes.
 * stdout: "V8D1", ordinal, decoded count, key bit, width, height, 16x16 luma
 * sum, actual sampled pixels. Original caller owns deadlines and child join. */
static int decode_mode(void) {
  vpx_codec_ctx_t decoder;
  vpx_codec_dec_cfg_t cfg = {1, WIDTH, HEIGHT};
  int initialized = 0, result = 1, first = 1;
  uint32_t previous = 0, requests = 0;
  unsigned char *encoded = malloc(MAX_ENCODED);
  if (!encoded || vpx_codec_dec_init(&decoder, vpx_codec_vp8_dx(), &cfg, 0)) goto done;
  initialized = 1;
  for (;;) {
    unsigned char input[16], output[32] = {'V','8','D','1'};
    size_t n = fread(input, 1, sizeof(input), stdin);
    if (n == 0 && feof(stdin)) { result = 0; break; }
    if (n != sizeof(input) || memcmp(input, "D8F1", 4)) goto done;
    uint32_t ordinal = load32(input + 4), length = load32(input + 8), key = load32(input + 12);
    if (++requests > MAX_FRAMES || length < 3 || length > MAX_ENCODED || key > 1 ||
        (!first && ordinal <= previous) || fread(encoded, 1, length, stdin) != length ||
        (uint32_t)((encoded[0] & 1) == 0) != key || (first && !key)) goto done;
    if (key) {
      vpx_codec_stream_info_t info;
      memset(&info, 0, sizeof(info)); info.sz = sizeof(info);
      if (vpx_codec_peek_stream_info(vpx_codec_vp8_dx(), encoded, length, &info) ||
          !info.is_kf || info.w != WIDTH || info.h != HEIGHT) goto done;
    }
    if (vpx_codec_decode(&decoder, encoded, length, NULL, 0)) goto done;
    int corrupt = 1;
    if (vpx_codec_control(&decoder, VP8D_GET_FRAME_CORRUPTED, &corrupt) || corrupt) goto done;
    vpx_codec_iter_t iterator = NULL;
    vpx_image_t *frame = vpx_codec_get_frame(&decoder, &iterator);
    if (!frame || frame->d_w != WIDTH || frame->d_h != HEIGHT ||
        frame->fmt != VPX_IMG_FMT_I420 || !frame->planes[0] || frame->stride[0] < WIDTH) goto done;
    uint32_t luma = 0, pixels = 0;
    for (unsigned y = 0; y < 16; ++y) for (unsigned x = 0; x < 16; ++x) {
      luma += frame->planes[0][y * (unsigned)frame->stride[0] + x]; ++pixels;
    }
    if (vpx_codec_get_frame(&decoder, &iterator) != NULL) goto done;
    save32(output + 4, ordinal); save32(output + 8, 1); save32(output + 12, key);
    save32(output + 16, WIDTH); save32(output + 20, HEIGHT);
    save32(output + 24, luma); save32(output + 28, pixels);
    if (fwrite(output, 1, sizeof(output), stdout) != sizeof(output) || fflush(stdout)) goto done;
    first = 0; previous = ordinal;
  }
done:
  if (result) fputs("live VP8 decoder refused input or decoding\n", stderr);
  if (initialized) vpx_codec_destroy(&decoder);
  free(encoded);
  return result;
}
int main(int argc, char **argv) {
  if (argc == 2 && strcmp(argv[1], "--decode-vp8") == 0) return decode_mode();
  int result = 1, fd = -1, initialized = 0;
  unsigned char *raw = NULL;
  vpx_codec_ctx_t encoder;
  vpx_codec_enc_cfg_t cfg;
  struct stat original, current;
  char *end = NULL;
  if (argc != 3) goto done;
  errno = 0;
  unsigned long count = strtoul(argv[2], &end, 10);
  if (errno || !argv[2][0] || *end || count < 1 || count > MAX_FRAMES)
    goto done;
  fd = open(argv[1], O_RDONLY | O_CLOEXEC | O_NOFOLLOW);
  if (fd < 0 || fstat(fd, &original) || !S_ISREG(original.st_mode) ||
      (original.st_mode & 077) || original.st_size != (off_t)count * RAW_BYTES)
    goto done;
  if (vpx_codec_enc_config_default(vpx_codec_vp8_cx(), &cfg, 0)) goto done;
  cfg.g_w = WIDTH;
  cfg.g_h = HEIGHT;
  cfg.g_threads = 1;
  cfg.g_timebase.num = 1;
  cfg.g_timebase.den = FPS;
  cfg.g_lag_in_frames = 0;
  cfg.g_pass = VPX_RC_ONE_PASS;
  cfg.rc_end_usage = VPX_VBR;
  cfg.rc_target_bitrate = 500;
  cfg.rc_2pass_vbr_maxsection_pct = 200;
  cfg.rc_buf_sz = 2000;
  cfg.rc_buf_initial_sz = 1500;
  cfg.rc_buf_optimal_sz = 1500;
  cfg.kf_mode = VPX_KF_AUTO;
  cfg.kf_min_dist = 0;
  cfg.kf_max_dist = FPS;
  if (vpx_codec_enc_init(&encoder, vpx_codec_vp8_cx(), &cfg, 0)) goto done;
  initialized = 1;
  if (vpx_codec_control(&encoder, VP8E_SET_CPUUSED, 8) ||
      vpx_codec_control(&encoder, VP8E_SET_ENABLEAUTOALTREF, 0)) goto done;
  raw = malloc(RAW_BYTES);
  if (!raw) goto done;
  uint32_t previous = 0, last_key = 0;
  int first = 1;
  for (;;) {
    unsigned char request[8], header[16] = {'V', '8', 'F', '1'};
    size_t n = fread(request, 1, sizeof(request), stdin);
    if (n == 0 && feof(stdin)) { result = 0; break; }
    if (n != sizeof(request)) goto done;
    uint32_t index = load32(request), flags = load32(request + 4);
    if (index >= count || flags > 1 || (!first && index <= previous)) goto done;
    if (fstat(fd, &current) || !same_file(&original, &current) ||
        !read_frame(fd, index, raw) || fstat(fd, &current) ||
        !same_file(&original, &current)) goto done;
    int force = first || flags == 1 || index - last_key >= FPS;
    vpx_image_t image;
    if (!vpx_img_wrap(&image, VPX_IMG_FMT_I420, WIDTH, HEIGHT, 1, raw)) goto done;
    if (vpx_codec_encode(&encoder, &image, index, 1,
                         force ? VPX_EFLAG_FORCE_KF : 0, VPX_DL_REALTIME))
      goto done;
    vpx_codec_iter_t iterator = NULL;
    const vpx_codec_cx_pkt_t *packet;
    unsigned emitted = 0;
    while ((packet = vpx_codec_get_cx_data(&encoder, &iterator))) {
      if (packet->kind != VPX_CODEC_CX_FRAME_PKT) continue;
      if (++emitted != 1 || packet->data.frame.pts != (vpx_codec_pts_t)index ||
          !packet->data.frame.sz || packet->data.frame.sz > MAX_ENCODED)
        goto done;
      uint32_t key = !!(packet->data.frame.flags & VPX_FRAME_IS_KEY);
      if (force && !key) goto done;
      if (key) last_key = index;
      save32(header + 4, index);
      save32(header + 8, (uint32_t)packet->data.frame.sz);
      save32(header + 12, key);
      if (fwrite(header, 1, sizeof(header), stdout) != sizeof(header) ||
          fwrite(packet->data.frame.buf, 1, packet->data.frame.sz, stdout) !=
              packet->data.frame.sz || fflush(stdout)) goto done;
    }
    if (emitted != 1) goto done;
    first = 0;
    previous = index;
  }
done:
  if (result) fputs("live VP8 fixture refused input or encoding\n", stderr);
  free(raw);
  if (initialized) vpx_codec_destroy(&encoder);
  if (fd >= 0) close(fd);
  return result;
}
