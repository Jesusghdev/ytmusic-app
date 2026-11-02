export interface PlayerState {
  title: string;
  artist: string;
  album: string;
  artwork: string;
  video_id: string;
  duration: number;
  position: number;
  playing: boolean;
  volume: number;
  updated_at: number;
}

export interface LyricLine {
  time: number;
  text: string;
}

export interface Lyrics {
  lines: LyricLine[];
  synced: boolean;
  instrumental: boolean;
  source: string;
  message: string;
}

export interface RemoteInfo {
  url: string;
  port: number;
}
