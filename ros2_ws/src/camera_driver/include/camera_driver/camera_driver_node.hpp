class CameraDriverNode : public rclcpp::Node
{
public:
    CameraDriverNode(
        const rclcpp::NodeOptions & options,
        int device_id,
        std::string topic,
        std::string frame_id,
        int fps
    );

private:
    void captureLoop();

    cv::VideoCapture cap_;
    image_transport::Publisher pub_;

    rclcpp::TimerBase::SharedPtr timer_;

    std::string topic_;
    std::string frame_id_;
    int fps_;
};